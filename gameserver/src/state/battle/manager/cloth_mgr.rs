use crate::state::battle::{
    cloth::{self, first_melody},
    deck::DeckManager,
};
use rand::rngs::StdRng;
use sonettobuf::{Fight, FightStep};
use std::collections::HashMap;

pub use cloth::{
    active_cloth_level, apply_cloth_power_delta, cloth_power_delta_for_operation,
    parse_cloth_recover_delta, seed_attacker_power_from_cloth,
};

#[derive(Debug, Clone, Default)]
pub struct ClothMgr {
    use_counts: HashMap<i32, usize>,
}

impl ClothMgr {
    pub fn reset(&mut self) {
        self.use_counts.clear();
    }

    pub fn execute_skill(
        &mut self,
        skill_id: i32,
        fight: &mut Fight,
        deck_mgr: &mut DeckManager,
        rng: &mut StdRng,
    ) -> anyhow::Result<Vec<FightStep>> {
        let cloth = active_cloth_level(fight).ok_or_else(|| anyhow::anyhow!("no cloth"))?;

        let cost_vec = if skill_id == cloth.skill1 {
            cloth.use_power1.clone()
        } else if skill_id == cloth.skill2 {
            cloth.use_power2.clone()
        } else {
            anyhow::bail!("cloth skill {skill_id} unimplemented")
        };

        let count = self.use_counts.entry(skill_id).or_insert(0);
        let cost = cost_vec.get(*count).copied().unwrap_or_else(|| cost_vec.last().copied().unwrap_or(0));
        *count += 1;

        cloth::apply_cloth_power_delta(fight, &cloth, -(cost as i32));

        let steps = match skill_id {
            30010201 => first_melody::universal_card(skill_id, deck_mgr),
            30010202 => first_melody::redeal_card(skill_id, fight, deck_mgr, rng),
            _ => anyhow::bail!("cloth skill {skill_id} unimplemented"),
        };

        Ok(steps)
    }
}
