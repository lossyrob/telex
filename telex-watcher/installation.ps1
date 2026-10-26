#Requires -Version 7.4
<#
.SYNOPSIS
Create, verify or preflight a non-secret Watcher installation record. Never installs binaries or starts a runtime.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('create', 'verify', 'preflight')][string]$Action,
    [string]$RecordPath,
    [string]$TelexExecutable,
    [string]$WatcherExecutable,
    [string]$WatcherRegistry,
    [string]$InstallRoot,
    [string]$BackendProfile,
    [string]$Sender,
    [ValidateRange(1, 86400)][int]$PollIntervalSeconds = 30,
    [string[]]$EnvironmentNames = @('TELEX_HOME')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not ($IsWindows -or $IsLinux)) { throw 'Watcher bootstrap supports Windows and Linux only.' }

function Assert-Absolute([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or -not [IO.Path]::IsPathFullyQualified($Path)) {
        throw "An absolute path is required: $Path"
    }
    [IO.Path]::GetFullPath($Path)
}

function Assert-AuthorityAncestors([IO.DirectoryInfo]$Directory) {
    while ($null -ne $Directory) {
        if ($Directory.Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw "Installation authority has a linked ancestor: $Directory"
        }
        if ($IsWindows) {
            $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
            $trusted = @($sid, 'S-1-5-18', 'S-1-5-32-544',
                'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
            $acl = Get-Acl -LiteralPath $Directory.FullName
            $descriptor = [Security.AccessControl.RawSecurityDescriptor]::new($acl.GetSecurityDescriptorBinaryForm(), 0)
            if ($null -eq $descriptor.DiscretionaryAcl -or
                $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trusted) {
                throw "Installation authority has an untrusted ancestor: $Directory"
            }
            foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
                # Creating siblings is harmless; deleting a child or changing ancestor authority is not.
                if ($rule.AccessControlType -eq 'Allow' -and
                    -not ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) -and
                    $rule.IdentityReference.Value -notin $trusted -and
                    ([long]$rule.FileSystemRights -band 0x500D0040) -ne 0) {
                    throw "Installation authority has a foreign-writable ancestor: $Directory"
                }
            }
        } else {
            $uid = & id -u
            if ($LASTEXITCODE -ne 0) { throw 'Cannot determine current uid.' }
            $owner = & stat -c '%u' -- $Directory.FullName
            if ($LASTEXITCODE -ne 0 -or $owner -notin @('0', $uid)) {
                throw "Installation authority has an untrusted ancestor: $Directory"
            }
            $mode = [int][IO.File]::GetUnixFileMode($Directory.FullName)
            if (($mode -band 18) -ne 0 -and ($mode -band 512) -eq 0) {
                throw "Installation authority has a foreign-writable ancestor: $Directory"
            }
        }
        $Directory = $Directory.Parent
    }
}

function Assert-Private([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Links and reparse points are not installation authority: $Path"
    }
    $ancestor = if ($item -is [IO.FileInfo]) { $item.Directory } else { $item.Parent }
    Assert-AuthorityAncestors $ancestor
    if ($IsWindows) {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        $acl = Get-Acl -LiteralPath $Path
        $descriptor = [Security.AccessControl.RawSecurityDescriptor]::new($acl.GetSecurityDescriptorBinaryForm(), 0)
        if ($null -eq $descriptor.DiscretionaryAcl) { throw "Installation authority has no restrictive DACL: $Path" }
        $owners = @($sid, 'S-1-5-32-544')
        if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $owners) {
            throw "Installation authority has a different owner: $Path"
        }
        $trusted = @($sid, 'S-1-5-18', 'S-1-5-32-544')
        foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
            if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -notin $trusted) {
                throw "Installation authority grants another principal access: $Path"
            }
        }
        if (-not $acl.AreAccessRulesProtected) {
            throw "Installation authority must have a protected owner-only ACL: $Path"
        }
    } else {
        $uid = & id -u
        if ($LASTEXITCODE -ne 0) { throw 'Cannot determine current uid.' }
        $owner = & stat -c '%u' -- $Path
        if ($LASTEXITCODE -ne 0 -or $owner -ne $uid) { throw "Installation authority has a different owner: $Path" }
        $mode = [IO.File]::GetUnixFileMode($Path)
        if (([int]$mode -band 63) -ne 0) { throw "Installation authority must be owner-only: $Path" }
    }
}

function New-PrivateDirectory([string]$Path) {
    if (Test-Path -LiteralPath $Path) {
        Assert-Private $Path
        if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw "Not a directory: $Path" }
        return
    }
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        throw "Create the parent directory explicitly before bootstrap: $parent"
    }
    Assert-AuthorityAncestors ([IO.DirectoryInfo]::new($parent))
    if ($IsWindows) {
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $acl.SetOwner($sid)
        $acl.SetAccessRuleProtection($true, $false)
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
        $acl.AddAccessRule($rule)
        [IO.FileSystemAclExtensions]::Create([IO.DirectoryInfo]::new($Path), $acl)
    } else {
        [IO.Directory]::CreateDirectory($Path, [IO.UnixFileMode]448) | Out-Null
    }
    Assert-Private $Path
}

function Read-JsonCommand([string]$Program, [string[]]$Arguments) {
    $output = & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Installation check failed: $Program $($Arguments -join ' ')" }
    ($output -join "`n") | ConvertFrom-Json -AsHashtable
}

function Write-PrivateFileExclusive([string]$Path, [byte[]]$Bytes) {
    $temporary = Join-Path (Split-Path -Parent $Path) "$([guid]::NewGuid()).tmp"
    try {
        $options = [IO.FileStreamOptions]::new()
        $options.Mode = [IO.FileMode]::CreateNew
        $options.Access = [IO.FileAccess]::Write
        if (-not $IsWindows) { $options.UnixCreateMode = [IO.UnixFileMode]384 }
        $file = [IO.FileStream]::new($temporary, $options)
        try {
            $file.Write($Bytes)
            $file.Flush($true)
        } finally { $file.Dispose() }
        if ($IsWindows) {
            $info = [IO.FileInfo]::new($temporary)
            $acl = [IO.FileSystemAclExtensions]::GetAccessControl($info, [Security.AccessControl.AccessControlSections]::Access)
            $acl.SetAccessRuleProtection($true, $true)
            [IO.FileSystemAclExtensions]::SetAccessControl($info, $acl)
        }
        Assert-Private $temporary
        [IO.File]::Move($temporary, $Path, $false)
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
    }
}

function Assert-LaunchEnvironment($Record) {
    foreach ($name in $Record.environmentNames) {
        if ($name -ne 'TELEX_HOME' -and [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($name))) {
            throw "Required launch environment is missing: $name"
        }
    }
}

function Assert-Record($Record, [switch]$Installed) {
    $keys = @('schemaVersion', 'configRoot', 'telexExecutable', 'watcherExecutable',
        'watcherRegistry', 'installRoot', 'backendProfile', 'sender', 'pollIntervalSeconds',
        'environmentNames', 'telexBuildId', 'watcherSha256', 'verificationToolSha256')
    if ($Record -isnot [System.Collections.IDictionary] -or
        @($Record.Keys).Count -ne $keys.Count -or
        @($Record.Keys | Where-Object { $_ -cnotin $keys }).Count -ne 0 -or
        ($Record.schemaVersion -isnot [int] -and $Record.schemaVersion -isnot [long]) -or
        $Record.schemaVersion -ne 1) {
        throw 'Invalid or unsupported installation record; use the Watcher bootstrap guide.'
    }
    foreach ($key in @('configRoot', 'telexExecutable', 'watcherExecutable', 'watcherRegistry', 'installRoot')) {
        if ($Record[$key] -isnot [string]) { throw "Record path must be a string: $key" }
        $null = Assert-Absolute $Record[$key]
    }
    if ($Record.configRoot -ne $configRoot) { throw 'Record configRoot does not match the current TELEX_HOME/home.' }
    if ($Record.verificationToolSha256 -isnot [string] -or $Record.verificationToolSha256 -cnotmatch '^[0-9a-f]{64}$') {
        throw 'Invalid verification tool hash in installation record.'
    }
    if ($Installed) {
        Assert-Private $verificationTool
        if ($Record.verificationToolSha256 -ne (Get-FileHash -LiteralPath $verificationTool -Algorithm SHA256).Hash.ToLowerInvariant()) {
            throw 'Verification tool changed; use the Watcher bootstrap from a trusted checkout.'
        }
    }
    if ($Record.backendProfile -isnot [string] -or
        [string]::IsNullOrEmpty($Record.backendProfile) -or
        $Record.backendProfile -match '[\s\p{Cc}]' -or
        [Text.Encoding]::UTF8.GetByteCount($Record.backendProfile) -gt 256 -or
        $Record.sender -isnot [string] -or
        [string]::IsNullOrEmpty($Record.sender) -or $Record.sender -match '[\s\p{Cc}]' -or
        [Text.Encoding]::UTF8.GetByteCount($Record.sender) -gt 1024 -or
        ($Record.pollIntervalSeconds -isnot [long] -and $Record.pollIntervalSeconds -isnot [int]) -or
        $Record.pollIntervalSeconds -lt 1 -or $Record.pollIntervalSeconds -gt 86400) {
        throw 'Invalid profile, sender or cadence in installation record.'
    }
    if ($Record.environmentNames -isnot [array] -or $Record.environmentNames.Count -eq 0) {
        throw 'environmentNames must be an array of launch environment names.'
    }
    foreach ($name in $Record.environmentNames) {
        if ($name -isnot [string] -or $name -notmatch '^[A-Za-z_][A-Za-z0-9_]*$') {
            throw 'Record contains an invalid environment name.'
        }
    }
    if ('TELEX_HOME' -notin $Record.environmentNames) { throw 'environmentNames must include TELEX_HOME.' }
    foreach ($name in @('TELEX_CONFIG', 'LOCALAPPDATA')) {
        if ((Test-Path "Env:$name") -and ($name -eq 'TELEX_CONFIG' -or $IsWindows) -and
            $name -notin $Record.environmentNames) {
            throw "Include the existing launch environment name: $name"
        }
        if (Test-Path "Env:$name") { $null = Assert-Absolute ([Environment]::GetEnvironmentVariable($name)) }
    }
    foreach ($path in @($Record.telexExecutable, $Record.watcherExecutable)) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Executable is missing: $path" }
    }
    if ($Record.watcherSha256 -ne (Get-FileHash -LiteralPath $Record.watcherExecutable -Algorithm SHA256).Hash.ToLowerInvariant()) {
        throw 'Watcher executable changed; create a new reviewed installation record.'
    }
    $version = (Read-JsonCommand $Record.telexExecutable @('--json', 'version', '--root', $Record.installRoot)).version
    if (-not $version.install.layout_detected -or
        $version.install.current_binary -ne $Record.telexExecutable -or
        $version.build_id -ne $Record.telexBuildId) {
        throw 'Telex must be the exact selected versioned binary in the recorded install root.'
    }
    $null = & $Record.telexExecutable copilot stall-detect --help
    if ($LASTEXITCODE -ne 0) { throw 'Telex lacks copilot stall-detect; use the existing upgrade surface.' }
    $watcherVersion = (& $Record.watcherExecutable --version) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Watcher executable verification failed.' }
    $profile = Read-JsonCommand $Record.telexExecutable @('--json', 'backend', 'show', $Record.backendProfile)
    if ($profile.kind -notin @('sqlite', 'postgres')) { throw 'Unsupported configured backend kind.' }
    if ($profile.kind -eq 'sqlite') { $null = Assert-Absolute $profile.target }
    $required = if ($profile.auth -eq 'entra') { 'entra' } else { $profile.kind }
    $skill = (& $Record.telexExecutable copilot skill) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Telex build feature verification failed.' }
    foreach ($features in @($skill, $watcherVersion)) {
        if (($required -eq 'entra' -and $features -notmatch 'entra auth available') -or
            $features -notmatch "backends \[[^\]]*\b$([regex]::Escape($profile.kind))\b") {
            throw 'Telex or Watcher build lacks the configured backend/authentication feature.'
        }
    }
    if ($profile.password_env -and $profile.password_env -notin $Record.environmentNames) {
        throw 'Include the profile password environment name in EnvironmentNames.'
    }
}

$configRoot = if (Test-Path Env:TELEX_HOME) {
    Assert-Absolute $env:TELEX_HOME
} else {
    Assert-Absolute (Join-Path $HOME '.telex')
}
if (-not $RecordPath) { $RecordPath = Join-Path $configRoot 'watcher/installation.json' }
$RecordPath = Assert-Absolute $RecordPath
$directory = Split-Path -Parent $RecordPath
$verificationTool = Join-Path $directory 'installation.ps1'
if ($RecordPath -eq $verificationTool) { throw 'RecordPath must not name the companion installation.ps1 tool.' }
if ($Action -ne 'create') {
    Assert-Private $directory
    Assert-Private $RecordPath
    if ((Get-Item -LiteralPath $RecordPath).Length -gt 65536) { throw 'Installation record exceeds 64 KiB.' }
    $text = Get-Content -LiteralPath $RecordPath -Raw
    $document = [Text.Json.JsonDocument]::Parse($text)
    try {
        if ($document.RootElement.ValueKind -ne 'Object') { throw 'Installation record must be a JSON object.' }
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
        foreach ($property in $document.RootElement.EnumerateObject()) {
            if (-not $names.Add($property.Name)) { throw 'Duplicate installation record field.' }
        }
    } finally { $document.Dispose() }
    $record = $text | ConvertFrom-Json -AsHashtable
    Assert-Record $record -Installed
    if ($Action -eq 'preflight') { Assert-LaunchEnvironment $record }
} else {
    if (Test-Path -LiteralPath $RecordPath) { throw 'Installation record already exists; verify it or choose an explicit new path.' }
    if (Test-Path -LiteralPath $verificationTool) { throw 'Companion verification tool already exists; bootstrap never overwrites it.' }
    foreach ($path in @($TelexExecutable, $WatcherExecutable, $WatcherRegistry, $InstallRoot)) {
        $null = Assert-Absolute $path
    }
    if (Test-Path -LiteralPath $WatcherRegistry) { throw 'Initial bootstrap requires a new registry; preserve the existing registry.' }
    $toolBytes = [IO.File]::ReadAllBytes($PSCommandPath)
    $version = (Read-JsonCommand $TelexExecutable @('--json', 'version', '--root', $InstallRoot)).version
    $record = [ordered]@{
        schemaVersion = 1
        configRoot = $configRoot
        telexExecutable = $TelexExecutable
        watcherExecutable = $WatcherExecutable
        watcherRegistry = $WatcherRegistry
        installRoot = $InstallRoot
        backendProfile = $BackendProfile
        sender = $Sender
        pollIntervalSeconds = $PollIntervalSeconds
        environmentNames = @($EnvironmentNames)
        telexBuildId = $version.build_id
        watcherSha256 = (Get-FileHash -LiteralPath $WatcherExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        verificationToolSha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($toolBytes)).ToLowerInvariant()
    }
    Assert-Record $record
    Assert-LaunchEnvironment $record
    New-PrivateDirectory $directory
    Write-PrivateFileExclusive $verificationTool $toolBytes
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($record | ConvertTo-Json -Depth 4))
    Write-PrivateFileExclusive $RecordPath $bytes
}
[Console]::Error.WriteLine("Installation record: $RecordPath")
$record | ConvertTo-Json -Depth 4
