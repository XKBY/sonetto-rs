use std::collections::HashSet;
use rand::{Rng, SeedableRng, rngs::StdRng, thread_rng};
use sonettobuf::{CardInfo, CardInfoPush, Fight, FightGroup};
use sqlx::SqlitePool;
use crate::error::AppError;
use super::draw::draw_deck_guaranteed_by_uid_with_rng;
use super::hand::{card_limit, purge_dead_entity_cards, refill_hand};
use super::pool::{build_enemy_deck, build_player_deck};

#[derive(Default, Debug, Clone)]
pub struct DeckManager {
    pub player_hand: Vec<CardInfo>,
    pub player_deck: Vec<CardInfo>,
    pub player_ex_deck: Vec<CardInfo>,
    pub enemy_hand: Vec<CardInfo>,
    pub enemy_deck: Vec<CardInfo>,
    pub enemy_ex_deck: Vec<CardInfo>,
}

impl DeckManager {
    pub fn purge_player_dead_cards(&mut self, alive_uids: &HashSet<i64>) {
        purge_dead_entity_cards(&mut self.player_hand, alive_uids);
        purge_dead_entity_cards(&mut self.player_ex_deck, alive_uids);
    }

    pub fn purge_enemy_dead_cards(&mut self, alive_uids: &HashSet<i64>) {
        purge_dead_entity_cards(&mut self.enemy_hand, alive_uids);
        purge_dead_entity_cards(&mut self.enemy_ex_deck, alive_uids);
    }

    pub fn refill_player_hand(&mut self, rng: &mut impl Rng, alive_uids: &HashSet<i64>, extra: usize, fight: &Fight) -> Vec<CardInfo> {
        refill_hand(rng, &mut self.player_hand, &mut self.player_deck, &mut self.player_ex_deck, alive_uids, extra, fight)
    }

    pub fn refill_enemy_hand(&mut self, rng: &mut impl Rng, alive_uids: &HashSet<i64>, fight: &Fight) -> Vec<CardInfo> {
        refill_hand(rng, &mut self.enemy_hand, &mut self.enemy_deck, &mut self.enemy_ex_deck, alive_uids, 0, fight)
    }

    pub async fn init_player(
        &mut self,
        pool: &SqlitePool,
        player_id: i64,
        fight_group: &FightGroup,
        max_ap: i32,
    ) -> Result<CardInfoPush, AppError> {
        let active_heroes: Vec<i64> = fight_group
            .hero_list.iter().copied().filter(|&u| u != 0).collect();
        let all_heroes: Vec<i64> = fight_group
            .hero_list.iter().chain(fight_group.sub_hero_list.iter())
            .copied().filter(|&u| u != 0).collect();

        let candidates = build_player_deck(pool, player_id, &active_heroes).await?;
        let full_deck = build_player_deck(pool, player_id, &all_heroes).await.unwrap_or_default();

        let has_support = !fight_group.sub_hero_list.is_empty();
        let opening_hand_size = card_limit(active_heroes.len(), has_support);
        let mut rng = thread_rng();
        let dealt = draw_deck_guaranteed_by_uid_with_rng(
            &candidates, &active_heroes, opening_hand_size, &mut rng,
        );

        self.player_hand = dealt.clone();
        self.player_deck = full_deck;
        self.player_ex_deck = vec![];

        Ok(CardInfoPush {
            card_group: dealt.clone(),
            deal_card_group: dealt,
            act_point: Some(max_ap),
            move_num: Some(0),
            before_cards: vec![],
            extra_move_act: Some(0),
            is_gm: Some(false),
        })
    }

    pub fn init_enemy(&mut self, fight: &Fight, seed: u64, monster_ids: &[i32]) {
        let enemy_deck_full = build_enemy_deck(monster_ids);
        let required_uids: Vec<i64> = monster_ids.iter().map(|&id| id as i64).collect();
        let opening_hand_size = card_limit(monster_ids.len(), false);
        let mut rng = thread_rng();
        let enemy_hand = draw_deck_guaranteed_by_uid_with_rng(
            &enemy_deck_full, &required_uids, opening_hand_size, &mut rng,
        );

        let mut ai_rng: StdRng = StdRng::seed_from_u64(seed);
        let attacker_uids: Vec<i64> = fight.attacker.as_ref().map(|a| {
            a.entitys.iter()
                .filter_map(|e| {
                    let uid = e.uid.unwrap_or(0);
                    if uid > 0 && e.current_hp.unwrap_or(0) > 0 { Some(uid) } else { None }
                })
                .collect()
        }).unwrap_or_default();

        let mut ai_deck = Vec::new();
        if !attacker_uids.is_empty() {
            if let Some(defender) = &fight.defender {
                for enemy in defender.entitys.iter().chain(defender.sub_entitys.iter()) {
                    let enemy_uid = enemy.uid.unwrap_or(0);
                    if enemy_uid >= 0 || enemy.current_hp.unwrap_or(0) <= 0 { continue; }
                    let Some(&skill_id) = enemy.skill_group1.first() else { continue; };
                    let target_uid = attacker_uids[ai_rng.gen_range(0..attacker_uids.len())];
                    ai_deck.push(CardInfo {
                        uid: Some(enemy_uid),
                        skill_id: Some(skill_id),
                        target_uid: Some(target_uid),
                        card_effect: Some(0),
                        temp_card: Some(false),
                        enchants: vec![],
                        card_type: Some(0),
                        hero_id: enemy.model_id,
                        status: Some(0),
                        extra_info: None,
                        energy: Some(0),
                        extra_infos: vec![],
                        area_red_or_blue: Some(0),
                        heat_id: Some(0),
                        music_note: None,
                    });
                }
            }
        }

        self.enemy_hand = enemy_hand;
        self.enemy_deck = ai_deck;
        self.enemy_ex_deck = vec![];
    }
}
