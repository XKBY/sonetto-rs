use super::fight_context::FightContext;

pub struct RoundContext<'a, 'ctx> {
    pub fight_ctx: &'a mut FightContext<'ctx>,
    pub round_index: i32,
}

impl<'a, 'ctx> RoundContext<'a, 'ctx> {
    pub fn new(fight_ctx: &'a mut FightContext<'ctx>, round_index: i32) -> Self {
        Self { fight_ctx, round_index }
    }

    pub fn sync(&mut self) {
        self.fight_ctx.sync();
    }

    pub fn on_round_end(&mut self) {
        self.fight_ctx.on_round_end();
    }
}
