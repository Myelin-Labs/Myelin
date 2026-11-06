use myelin_session::SessionConfig;
use myelin_session_transfer::{SourceBindings, TransferExecutor, TransferState};

fn main() {
    let run = || -> Result<(), String> {
        let encoded = std::env::args().nth(1).ok_or("missing genesis config")?;
        if !encoded.is_ascii() || !encoded.len().is_multiple_of(2) || encoded.len() > 4096 {
            return Err("invalid genesis config argument".into());
        }
        let bytes = (0..encoded.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&encoded[i..i + 2], 16).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let cfg = SessionConfig::decode(&bytes).map_err(|e| e.to_string())?;
        myelin_session_worker::wire::serve_stdio(cfg.application_profile.commitment(), |snapshot| {
            let state = TransferState::decode(snapshot).map_err(|e| e.to_string())?;
            let sources = SourceBindings::no_sources(state.policy()).map_err(|e| e.to_string())?;
            TransferExecutor::restore(&cfg, snapshot, sources).map_err(|e| e.to_string())
        })
    };
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(70);
    }
}
