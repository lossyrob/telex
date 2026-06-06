use anyhow::{anyhow, bail, Result};
use serde_json::json;

use crate::backend::available_kinds;
use crate::cli::{BackendAddArgs, BackendCmd, Ctx};
use crate::output::emit;
use crate::profiles::{self, BackendProfile};

pub async fn run(ctx: &Ctx, cmd: BackendCmd) -> Result<i32> {
    match cmd {
        BackendCmd::Add(a) => add(ctx, a),
        BackendCmd::List => list(ctx),
        BackendCmd::Show { name } => show(ctx, &name),
        BackendCmd::Remove { name } => remove(ctx, &name),
        BackendCmd::Default { name } => set_default(ctx, &name),
        BackendCmd::Kinds => kinds(ctx),
    }
}

fn add(ctx: &Ctx, args: BackendAddArgs) -> Result<i32> {
    if args.sqlite && args.postgres.is_some() {
        bail!("specify only one of --sqlite or --postgres");
    }
    if args.entra && args.postgres.is_none() {
        bail!("--entra applies to a --postgres backend");
    }
    if args.entra && (args.password_env.is_some() || args.password_command.is_some()) {
        bail!("--entra cannot be combined with --password-env/--password-command");
    }

    let profile = if args.sqlite {
        BackendProfile {
            kind: "sqlite".into(),
            path: args.path.clone(),
            url: None,
            auth: None,
            password_env: None,
            password_command: None,
            schema: None,
            entra_cred: None,
            entra_scope: None,
        }
    } else if let Some(conn) = &args.postgres {
        let auth = if args.entra { "entra" } else { "password" };
        BackendProfile {
            kind: "postgres".into(),
            path: None,
            url: Some(conn.clone()),
            auth: Some(auth.into()),
            password_env: args.password_env.clone(),
            password_command: args.password_command.clone(),
            schema: args.schema.clone(),
            entra_cred: if args.entra {
                args.entra_cred.clone()
            } else {
                None
            },
            entra_scope: if args.entra {
                args.entra_scope.clone()
            } else {
                None
            },
        }
    } else {
        bail!("specify a backend kind: --sqlite or --postgres <connection-string>");
    };

    let mut cfg = profiles::load()?;
    let first = cfg.backends.is_empty();
    let made_default = args.default || first;
    cfg.backends.insert(args.name.clone(), profile.clone());
    if made_default {
        cfg.default = Some(args.name.clone());
    }
    profiles::save(&cfg)?;

    let out = json!({
        "added": args.name,
        "kind": profile.kind,
        "target": profile.target(),
        "default": made_default,
    });
    emit(ctx.fmt, &out, || {
        println!(
            "added backend '{}' ({}) -> {}{}",
            args.name,
            profile.kind,
            profile.target(),
            if made_default { "  [default]" } else { "" }
        );
    });
    Ok(0)
}

fn list(ctx: &Ctx) -> Result<i32> {
    let cfg = profiles::load()?;
    let entries: Vec<_> = cfg
        .backends
        .iter()
        .map(|(name, p)| {
            json!({
                "name": name,
                "kind": p.kind,
                "default": cfg.default.as_deref() == Some(name.as_str()),
                "auth": p.auth,
                "target": p.target(),
            })
        })
        .collect();
    let out = json!({ "count": entries.len(), "default": cfg.default, "backends": entries });
    emit(ctx.fmt, &out, || {
        if entries.is_empty() {
            println!("(no backends configured; an implicit 'default' sqlite store is used)");
            return;
        }
        #[allow(clippy::print_literal)]
        {
            println!("{:<14} {:<9} {:<8} {}", "NAME", "KIND", "DEFAULT", "TARGET");
        }
        for e in &entries {
            println!(
                "{:<14} {:<9} {:<8} {}",
                e["name"].as_str().unwrap_or(""),
                e["kind"].as_str().unwrap_or(""),
                if e["default"].as_bool().unwrap_or(false) {
                    "*"
                } else {
                    ""
                },
                e["target"].as_str().unwrap_or("")
            );
        }
    });
    Ok(0)
}

fn show(ctx: &Ctx, name: &str) -> Result<i32> {
    let cfg = profiles::load()?;
    let p = cfg
        .backends
        .get(name)
        .ok_or_else(|| anyhow!("no backend named '{name}'"))?;
    let password_source = if p.auth.as_deref() == Some("entra") {
        "entra"
    } else if p.password_env.is_some() {
        "env"
    } else if p.password_command.is_some() {
        "command"
    } else if p.kind == "postgres" {
        "in-connection-string"
    } else {
        "none"
    };
    let out = json!({
        "name": name,
        "kind": p.kind,
        "default": cfg.default.as_deref() == Some(name),
        "target": p.target(),
        "auth": p.auth,
        "schema": p.schema,
        "password_source": password_source,
        "password_env": p.password_env,
        "password_command": p.password_command,
    });
    emit(ctx.fmt, &out, || {
        println!("name      {name}");
        println!("kind      {}", p.kind);
        println!("target    {}", p.target());
        if p.kind == "postgres" {
            println!("auth      {}", p.auth.as_deref().unwrap_or("password"));
            println!("password  {password_source}");
            if let Some(s) = &p.schema {
                println!("schema    {s}");
            }
        }
    });
    Ok(0)
}

fn remove(ctx: &Ctx, name: &str) -> Result<i32> {
    let mut cfg = profiles::load()?;
    if cfg.backends.remove(name).is_none() {
        bail!("no backend named '{name}'");
    }
    let cleared_default = cfg.default.as_deref() == Some(name);
    if cleared_default {
        cfg.default = cfg.backends.keys().next().cloned();
    }
    profiles::save(&cfg)?;
    let out = json!({ "removed": name, "new_default": cfg.default });
    emit(ctx.fmt, &out, || {
        println!("removed backend '{name}'");
        if cleared_default {
            match &cfg.default {
                Some(d) => println!("default is now '{d}'"),
                None => println!("no default backend set (implicit sqlite will be used)"),
            }
        }
    });
    Ok(0)
}

fn set_default(ctx: &Ctx, name: &str) -> Result<i32> {
    let mut cfg = profiles::load()?;
    if !cfg.backends.contains_key(name) {
        bail!("no backend named '{name}'");
    }
    cfg.default = Some(name.to_string());
    profiles::save(&cfg)?;
    let out = json!({ "default": name });
    emit(ctx.fmt, &out, || {
        println!("default backend is now '{name}'")
    });
    Ok(0)
}

fn kinds(ctx: &Ctx) -> Result<i32> {
    let kinds = available_kinds();
    let out = json!({ "kinds": kinds });
    emit(ctx.fmt, &out, || {
        println!("backend kinds in this build: {}", kinds.join(", "));
    });
    Ok(0)
}
