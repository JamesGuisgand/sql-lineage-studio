use std::{env, fs, io::{self, Read}, process};
use sql_lineage_rs::analyze;

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut format = "json";
    let mut dialect = "generic";
    let mut impact = None;
    let mut filename = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--format" | "--dialect" | "--impact" => {
                let flag = &args[index];
                index += 1;
                let val = args.get(index).ok_or(format!("Option {flag} sans valeur"))?;
                match flag.as_str() {
                    "--format" => format = val,
                    "--dialect" => dialect = val,
                    "--impact" => impact = Some(val.as_str()),
                    _ => unreachable!(),
                }
            }
            "--help" | "-h" => {
                println!("Usage: sql-lineage-rs --format json|dot|text --dialect generic|postgres|mysql|sqlite [--impact TABLE] file.sql|-");
                return Ok(());
            }
            name if filename.is_none() => filename = Some(name),
            _ => return Err("Arguments en trop".into()),
        }
        index += 1;
    }
    if !matches!(format, "json" | "dot" | "text") { return Err("Format invalide".into()); }
    let filename = filename.ok_or("Nom de fichier SQL manquant")?;
    let sql = if filename == "-" {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input).map_err(|e| e.to_string())?;
        input
    } else {
        fs::read_to_string(filename).map_err(|e| e.to_string())?
    };
    let report = analyze(&sql, dialect, impact)?;
    match format {
        "dot" => print!("{}", report.dot),
        "text" => for (i, stmt) in report.statements.iter().enumerate() {
            println!("#{} type={} cible={} sources={}", i+1, stmt.kind,
                     stmt.target.as_deref().unwrap_or("(aucune)"), stmt.sources.join(", "));
        },
        _ => println!("{}", serde_json::to_string(&report).map_err(|e| e.to_string())?),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Erreur : {error}");
        process::exit(1);
    }
}
