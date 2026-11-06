mod application;
use application::{profile, Application};
use myelin_session::{ExecutionOutcome, FrameInput, Hash32, SessionHandoff, TransitionExecutor};
use std::io::{Read, Write};

#[derive(Clone)]
struct FaultExecutor(Application);
impl TransitionExecutor for FaultExecutor {
    fn state_root(&self) -> Hash32 {
        self.0.root
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        self.0.snapshot()
    }
    fn execute_block_with_context(
        &mut self,
        context: &myelin_session::ProtocolEffectsContext,
        input: &FrameInput,
        handoffs: &[myelin_session::SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        context.require_authority_metadata([99; 32])?;
        self.execute_block(context.height, input, handoffs, transactions)
    }
    fn execute_block(
        &mut self,
        height: u64,
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        txs: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        match input.payload.as_slice() {
            b"spin" => loop {
                std::hint::spin_loop();
            },
            b"sleep" => std::thread::sleep(std::time::Duration::from_secs(30)),
            b"flood" => loop {
                std::io::stdout().write_all(&[7; 8192]).unwrap();
            },
            b"stderr" => loop {
                std::io::stderr().write_all(&[7; 8192]).unwrap();
            },
            b"exit" => std::process::exit(73),
            b"allocate" => {
                let mut data = Vec::<u8>::new();
                if data.try_reserve_exact(1024 * 1024 * 1024).is_err() {
                    std::process::exit(72);
                }
                data.resize(1024 * 1024 * 1024, 7);
                std::hint::black_box(&data);
                std::process::exit(74);
            }
            b"spawn" => {
                if std::process::Command::new("/bin/true").spawn().is_ok() {
                    std::process::exit(75);
                }
                if std::thread::Builder::new().spawn(|| ()).is_ok() {
                    std::process::exit(76);
                }
            }
            b"work" => {
                let mut data = vec![1u8; 4 * 1024 * 1024];
                for _ in 0..10 {
                    for byte in &mut data {
                        *byte = byte.wrapping_add(1);
                    }
                }
                std::hint::black_box(data);
            }
            b"bad-packet" => {
                std::io::stdout().write_all(b"not a worker reply").unwrap();
                std::process::exit(0);
            }
            _ => {}
        }
        self.0.execute_block(height, input, handoffs, txs)
    }
}

fn main() {
    if let Some(mode) = std::env::args().nth(1) {
        let mut packet = Vec::new();
        std::io::stdin().take(1024 * 1024).read_to_end(&mut packet).unwrap();
        let request: myelin_session_worker::wire::Request = myelin_session_worker::wire::decode(&packet, 1024 * 1024).unwrap();
        let reply = myelin_session_worker::wire::Reply {
            request_hash: if mode == "stale" { [0; 32] } else { *blake3::hash(&packet).as_bytes() },
            result: Ok(myelin_session_worker::wire::StateResult {
                pre_root: if mode == "wrong-state" { [9; 32] } else { request.pre_root },
                state_root: request.pre_root,
                snapshot: request.snapshot,
                outcome: None,
            }),
        };
        std::io::stdout().write_all(&myelin_session_worker::wire::encode(&reply, 1024 * 1024).unwrap()).unwrap();
        return;
    }
    if let Err(error) =
        myelin_session_worker::wire::serve_stdio(profile().commitment(), |bytes| Application::restore(bytes).map(FaultExecutor))
    {
        eprintln!("{error}");
        std::process::exit(70);
    }
}
