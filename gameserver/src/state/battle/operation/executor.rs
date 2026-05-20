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
        Ok(CardOpType::MoveCard) => {
            let uid = state.selected_cards.get(state.used_cards.len())
                .and_then(|c| c.uid).unwrap_or(0);
            ctx.on_move_card(uid);
            Ok(FightStep::default())
        }
        Ok(CardOpType::PlayCard)
        | Ok(CardOpType::AssistBoss)
        | Ok(CardOpType::PlayerFinisherSkill)
        | Ok(CardOpType::BloodPool) => {
            let op_index = state.used_cards.len();
            let card = state.selected_cards.get(op_index);
            let uid = card.and_then(|c| c.uid).unwrap_or(0);
            let skill_id = card.and_then(|c| c.skill_id).unwrap_or(0);
            let target_id = oper.to_id.unwrap_or(0);
            let result = crate::state::battle::card::executor::play_card(executor, rng, ctx, state, oper).await;
            ctx.on_use_card(uid, target_id, skill_id);
            result
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
