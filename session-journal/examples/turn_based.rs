#[path = "support/command_games.rs"]
mod games;
fn main() -> games::AnyResult<()> {
    games::main_for(games::Kind::Turns)
}
