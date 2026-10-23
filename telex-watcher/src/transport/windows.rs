//! Uses the daemon's current-token SID / protected owner-only SDDL pattern,
//! including aligned TOKEN_USER storage and remote-rejecting first instances.

use super::{connect_error, TransportError, TransportErrorCode, MAX_CLIENTS};
use std::ffi::{c_void, OsStr};
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};
use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_PIPE_BUSY, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    SECURITY_IDENTIFICATION,
};
use windows_sys::Win32::System::Pipes::{
    CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(super) struct Listener {
    name: String,
    next: Option<NamedPipeServer>,
}

impl Listener {
    pub(super) fn bind(key: &str) -> io::Result<Self> {
        let name = pipe_name(key)?;
        let next = Some(create_pipe(&name, true)?);
        Ok(Self { name, next })
    }

    pub(super) async fn accept(&mut self) -> io::Result<NamedPipeServer> {
        // Keep the connected instance alive until its successor is created:
        // the name never disappears while the owning listener exists.
        let server = self.next.as_ref().expect("listening pipe instance");
        server.connect().await?;
        let next = create_pipe(&self.name, false)?;
        Ok(self.next.replace(next).expect("connected pipe instance"))
    }
}

fn pipe_name(key: &str) -> io::Result<String> {
    let sid = current_sid()?;
    let user = super::hash_key(sid.as_bytes());
    Ok(format!(r"\\.\pipe\telex-watcher-v1-{user}-{key}"))
}

pub(super) async fn connect(key: &str) -> Result<NamedPipeClient, TransportError> {
    let name = pipe_name(key).map_err(|error| connect_error(&error))?;
    for _ in 0..80 {
        match ClientOptions::new()
            // A squatting same-user server must not gain an impersonation token.
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(&name)
        {
            Ok(client) => return Ok(client),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(error) => return Err(connect_error(&error)),
        }
    }
    Err(TransportError::new(
        TransportErrorCode::Busy,
        "All local management pipe instances remained busy; no command was submitted.",
    ))
}

pub(super) fn verify_client(connection: &NamedPipeServer) -> Result<(), TransportError> {
    let mut pid = 0;
    let ok = unsafe { GetNamedPipeClientProcessId(connection.as_raw_handle() as HANDLE, &mut pid) };
    verify_pid_result(ok, pid)
}

pub(super) fn verify_server(connection: &NamedPipeClient) -> Result<(), TransportError> {
    let mut pid = 0;
    let ok = unsafe { GetNamedPipeServerProcessId(connection.as_raw_handle() as HANDLE, &mut pid) };
    verify_pid_result(ok, pid)
}

fn verify_pid_result(ok: i32, pid: u32) -> Result<(), TransportError> {
    let verification = || -> io::Result<()> {
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
        if process.0 == 0 {
            return Err(io::Error::last_os_error());
        }
        if process_sid(process.0)? != current_sid()? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "foreign SID",
            ));
        }
        Ok(())
    };
    verification().map_err(|_| {
        TransportError::new(
            TransportErrorCode::Unauthorized,
            "The named-pipe peer could not be verified as the same OS user.",
        )
    })
}

fn create_pipe(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    let security = owner_only_security()?;
    let name = wide(OsStr::new(name));
    let mode = PIPE_ACCESS_DUPLEX
        | FILE_FLAG_OVERLAPPED
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            (MAX_CLIENTS + 2) as u32,
            8192,
            8192,
            0,
            &security.attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    unsafe { NamedPipeServer::from_raw_handle(handle as _) }
}

fn current_sid() -> io::Result<String> {
    process_sid(unsafe { GetCurrentProcess() })
}

fn process_sid(process: HANDLE) -> io::Result<String> {
    let mut token = 0;
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut needed = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![
        MaybeUninit::<TOKEN_USER>::uninit();
        (needed as usize).div_ceil(size_of::<TOKEN_USER>())
    ];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0;
    while unsafe { *sid.add(length) } != 0 {
        length += 1;
    }
    let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid, length) });
    unsafe { LocalFree(sid.cast()) };
    Ok(result)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}

struct Security {
    attributes: SECURITY_ATTRIBUTES,
    descriptor: *mut c_void,
}

impl Drop for Security {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor) };
    }
}

fn owner_only_security() -> io::Result<Security> {
    let sid = current_sid()?;
    let sddl = wide(OsStr::new(&format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})")));
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(Security {
        attributes: SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        },
        descriptor,
    })
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        GetAce, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        GetSecurityDescriptorOwner, ACCESS_ALLOWED_ACE, SE_DACL_PROTECTED,
    };

    #[tokio::test]
    async fn first_instance_flag_refuses_a_preexisting_squatter() {
        use tokio::net::windows::named_pipe::ServerOptions;

        let fixture = super::super::tests::Fixture::new();
        let key = super::super::endpoint_key(&fixture.registry).unwrap();
        let name = pipe_name(&key).unwrap();
        let squatter = ServerOptions::new()
            .first_pipe_instance(true)
            .create(name)
            .unwrap();
        assert!(Listener::bind(&key).is_err());
        drop(squatter);
        assert!(Listener::bind(&key).is_ok());
    }

    #[test]
    fn descriptor_grants_exactly_the_current_token_user_and_nobody_else() {
        let security = owner_only_security().unwrap();
        let mut control = 0;
        let mut revision = 0;
        assert_ne!(
            unsafe {
                GetSecurityDescriptorControl(security.descriptor, &mut control, &mut revision)
            },
            0
        );
        assert_ne!(control & SE_DACL_PROTECTED, 0);
        assert_eq!(security.attributes.bInheritHandle, 0);
        let mut owner = std::ptr::null_mut();
        let mut defaulted = 0;
        assert_ne!(
            unsafe { GetSecurityDescriptorOwner(security.descriptor, &mut owner, &mut defaulted) },
            0
        );
        let mut owner_string = std::ptr::null_mut();
        assert_ne!(
            unsafe { ConvertSidToStringSidW(owner, &mut owner_string) },
            0
        );
        let mut length = 0;
        while unsafe { *owner_string.add(length) } != 0 {
            length += 1;
        }
        let owner_sid =
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(owner_string, length) });
        unsafe { LocalFree(owner_string.cast()) };
        assert_eq!(owner_sid, current_sid().unwrap());
        let mut present = 0;
        let mut acl = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(
                    security.descriptor,
                    &mut present,
                    &mut acl,
                    &mut defaulted,
                )
            },
            0
        );
        assert_ne!(present, 0);
        assert!(!acl.is_null());
        assert_eq!(unsafe { (*acl).AceCount }, 1);
        let mut ace = std::ptr::null_mut();
        assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
        let ace = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(ace.Header.AceType, 0); // ACCESS_ALLOWED_ACE_TYPE
        assert_eq!(ace.Mask, 0x1000_0000); // GENERIC_ALL
        assert_ne!(
            unsafe {
                windows_sys::Win32::Security::EqualSid(
                    owner,
                    (&ace.SidStart as *const u32).cast_mut().cast(),
                )
            },
            0
        );
        assert_eq!(verify_pid_result(1, 0).unwrap_err().code(), "unauthorized");
    }
}
