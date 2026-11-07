//! Execute the original CKB program against retained inputs/dependencies. This
//! reports script validity separately from node inclusion and since maturity.
use ckb_testtool::context::Context;
use ckb_types::{bytes::Bytes, packed, prelude::*};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{fs, io::Read, path::Path};
const MAX: usize = 32 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cell {
    out_point: String,
    output: String,
    data: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    format: String,
    genesis: String,
    program_hash: String,
    transaction: String,
    cells: Vec<Cell>,
}
fn bytes(value: &str) -> Result<Vec<u8>, String> {
    let value = value.strip_prefix("0x").ok_or("hex must start with 0x")?;
    if !value.is_ascii() || value.len() % 2 != 0 || value.len() > MAX * 2 {
        return Err("invalid hex length".into());
    }
    (0..value.len()).step_by(2).map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}
fn hex(value: &[u8]) -> String {
    let mut out = String::from("0x");
    for b in value {
        use std::fmt::Write;
        write!(&mut out, "{b:02x}").unwrap();
    }
    out
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let mut output = vec![];
    fs::File::open(path).map_err(|e| e.to_string())?.take(MAX as u64 + 1).read_to_end(&mut output).map_err(|e| e.to_string())?;
    if output.len() > MAX {
        return Err("input exceeds byte ceiling".into());
    }
    Ok(output)
}
fn verify(path: &Path, expected: &str, genesis: &str) -> Result<Value, String> {
    let input: Input = serde_json::from_slice(&read(path)?).map_err(|e| e.to_string())?;
    if input.format != "myelin-ckb-microgame-replay-v1"
        || input.program_hash != expected
        || input.genesis != genesis
        || bytes(expected)?.len() != 32
        || bytes(genesis)?.len() != 32
        || input.cells.len() > 128
    {
        return Err("independent program/genesis/schema pin differs".into());
    }
    let tx = packed::Transaction::from_slice(&bytes(&input.transaction)?).map_err(|e| e.to_string())?.into_view();
    let mut context = Context::default();
    context.set_capture_debug(true);
    for cell in input.cells {
        let out_point = packed::OutPoint::from_slice(&bytes(&cell.out_point)?).map_err(|e| e.to_string())?;
        let output = packed::CellOutput::from_slice(&bytes(&cell.output)?).map_err(|e| e.to_string())?;
        let data: Bytes = bytes(&cell.data)?.into();
        if context.cells.contains_key(&out_point) {
            return Err("duplicate resolved cell".into());
        }
        context.create_cell_with_out_point(out_point, output, data);
    }
    for input in tx.inputs() {
        if !context.cells.contains_key(&input.previous_output()) {
            return Err("unresolved input".into());
        }
    }
    for dep in tx.cell_deps() {
        let (_, data) = context.cells.get(&dep.out_point()).ok_or("unresolved dependency")?;
        if dep.dep_type().as_slice() == [1] {
            let points = packed::OutPointVec::from_slice(data).map_err(|e| e.to_string())?;
            for point in points {
                if !context.cells.contains_key(&point) {
                    return Err("unresolved dep-group child".into());
                }
            }
        }
    }
    let actual_program = bytes(expected)?;
    if !context.cells.values().any(|(_, data)| packed::CellOutput::calc_data_hash(data).as_slice() == actual_program) {
        return Err("original program artifact absent".into());
    }
    let result = context.verify_tx(&tx, 10_000_000);
    Ok(match result {
        Ok(cycles) => {
            json!({"valid":true,"cycles":cycles,"tx_hash":hex(tx.hash().as_slice()),"witness_hash":hex(tx.witness_hash().as_slice()),"assurance":"CKB-VM script execution; inclusion/maturity separate"})
        }
        Err(error) => {
            json!({"valid":false,"error":error.to_string(),"tx_hash":hex(tx.hash().as_slice()),"assurance":"CKB-VM script rejection; inclusion/maturity separate"})
        }
    })
}
fn block(path: &Path, expected: &str) -> Result<Value, String> {
    let input: ckb_jsonrpc_types::BlockView = serde_json::from_slice(&read(path)?).map_err(|e| e.to_string())?;
    let declared = format!("{:#x}", input.header.hash);
    let view: ckb_types::core::BlockView = input.into();
    if declared != expected
        || hex(view.hash().as_slice()) != expected
        || view.header().transactions_root() != view.calc_transactions_root()
        || view.header().proposals_hash() != view.calc_proposals_hash()
        || view.header().extra_hash() != view.calc_extra_hash().extra_hash()
    {
        return Err("block identity/transaction-witness/proposal/uncle root differs".into());
    }
    Ok(
        json!({"valid":true,"block_hash":expected,"number":view.number(),"parent_hash":hex(view.header().parent_hash().as_slice()),"transactions":view.transactions().iter().map(|tx|hex(tx.hash().as_slice())).collect::<Vec<_>>(),"assurance":"retained header and full block body rehashed; canonical anchor/consensus separate"}),
    )
}
fn run() -> Result<Value, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [op, path, program, genesis] if op == "verify" => verify(Path::new(path), program, genesis),
        [op, path, expected] if op == "verify-block" => block(Path::new(path), expected),
        _ => Err("usage: verify PROOF EXPECTED_PROGRAM_HASH EXPECTED_GENESIS | verify-block BLOCK EXPECTED_BLOCK_HASH".into()),
    }
}
fn main() {
    match run() {
        Ok(value) => {
            println!("{value}");
            if value["valid"] != true {
                std::process::exit(1);
            }
        }
        Err(error) => {
            println!("{}", json!({"valid":false,"host_input_error":error}));
            std::process::exit(2);
        }
    }
}
