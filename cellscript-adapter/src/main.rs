// SPDX-License-Identifier: MIT

use myelin_cellscript_adapter::{build_and_attest, CellScriptInstallation, CompileRequest};
use std::{env, fs, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("build-attest") => {
            let source_root = required_path(&mut args, "source-root")?;
            let output = required_path(&mut args, "output")?;
            reject_extra(args)?;
            let (binary, attestation) = build_and_attest(&source_root)?;
            fs::write(output, serde_json::to_vec_pretty(&attestation)?)?;
            println!("{}", binary.display());
        }
        Some("verify") => {
            let binary = required_path(&mut args, "binary")?;
            let attestation = required_path(&mut args, "attestation")?;
            reject_extra(args)?;
            CellScriptInstallation::verify(binary, &attestation)?;
        }
        Some("compile") => {
            let binary = required_path(&mut args, "binary")?;
            let attestation = required_path(&mut args, "attestation")?;
            let source = required_path(&mut args, "source")?;
            let artifact = required_path(&mut args, "artifact")?;
            let entry_action = args.next();
            reject_extra(args)?;
            let installation = CellScriptInstallation::verify(binary, &attestation)?;
            let result = installation.compile(CompileRequest {
                source: &source,
                artifact: &artifact,
                target_profile: "ckb",
                entry_action: entry_action.as_deref(),
            })?;
            println!("{}", serde_json::to_string(&result)?);
        }
        _ => return Err("usage: myelin-cellscript-adapter build-attest <source-root> <output> | verify <binary> <attestation> | compile <binary> <attestation> <source> <artifact> [entry-action]".into()),
    }
    Ok(())
}

fn required_path(args: &mut impl Iterator<Item = String>, field: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    args.next().map(PathBuf::from).ok_or_else(|| format!("missing {field}").into())
}

fn reject_extra(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn std::error::Error>> {
    if args.next().is_some() {
        return Err("unexpected extra argument".into());
    }
    Ok(())
}
