use anyhow::Result;
use rand::rngs::StdRng;
use sonettobuf::{BeginRoundOper, FightStep, fight_step};

use crate::state::battle::{
    card::CardOpType,
    context::FightContext,
    fight_step::ActEffectBuilder,
    round::RoundState,
    skill::SkillExecutor,
};

pub async fn execute_operation(
    executor: &mut SkillExecutor,
    rng: &mut StdRng,
    ctx: &mut FightContext<'_>,
    state: &mut RoundState,
    oper: BeginRoundOper,
) -> Result<FightStep> {
    ctx.managers.buff_mgr.clear_step_deleted_buff_ids();
    let op = CardOpType::try_from(oper.oper_type.unwrap_or(0));
    match op {
        Ok(CardOpType::PlayCard)
        | Ok(CardOpType::MoveCard)
        | Ok(CardOpType::AssistBoss)
        | Ok(CardOpType::PlayerFinisherSkill)
        | Ok(CardOpType::BloodPool) => {
            if matches!(op, Ok(CardOpType::MoveCard)) && oper.to_id.unwrap_or(0) == 0 {
                return Ok(FightStep::default());
            }
            crate::state::battle::card::executor::play_card(executor, rng, ctx, state, oper).await
        }
        Ok(CardOpType::SimulateDissolveCard) => {
            let dissolve_index = (oper.param1.unwrap_or(1) - 1) as usize;
            if dissolve_index < state.selected_cards.len() {
                state.selected_cards.remove(dissolve_index);
            }
            Ok(FightStep {
                act_type: Some(fight_step::ActType::Effect.into()),
                act_effect: vec![ActEffectBuilder::cards_push(state.selected_cards.clone(), Some(1))],
                ..Default::default()
            })
        }
        _ => Ok(FightStep::default()),
    }
}
