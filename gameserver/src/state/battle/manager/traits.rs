#[allow(dead_code)]
pub trait Manager {
    fn on_round_start(&mut self) {}
    fn on_round_end(&mut self) {}
    fn on_battle_end(&mut self) {}
}
