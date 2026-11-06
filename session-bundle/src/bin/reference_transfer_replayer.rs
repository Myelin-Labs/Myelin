//! Reference application runner; never selects rules from a newer checkout.
use myelin_session::SessionError;
use myelin_session_bundle::runner::{replay_output, replayer_input};
fn main() {
    let result = (|| {
        let (store, session) = replayer_input()?;
        let config = store.config(session)?.clone();
        let receipt = store.replay_current_process(session, move |bytes, store| {
            myelin_session_transfer::restore_executor(&config, bytes, store).map_err(|e| SessionError::Replay(e.to_string()))
        })?;
        replay_output(&store, session, receipt)
    })();
    match result {
        Ok(result) => println!("{}", serde_json::to_string(&result).expect("runner result is JSON")),
        Err(error) => {
            eprintln!("{}", serde_json::to_string(&error).expect("runner error is JSON"));
            std::process::exit(2);
        }
    }
}
