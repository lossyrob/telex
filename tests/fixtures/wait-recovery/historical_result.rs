    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("telex: {e:#}");
            1
        }
    }
