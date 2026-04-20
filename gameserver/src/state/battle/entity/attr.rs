use config::configs;
use database::{db::game::equipment::Equipment, models::game::heros::HeroData};

use sonettobuf::HeroAttribute;

pub struct Attr;

impl Attr {
    pub fn get(hero_data: &HeroData, equip: Option<&Equipment>) -> HeroAttribute {
        let r = &hero_data.record;

        let mut hp = ((r.base_hp as f32) * 1.0986541).round() as i32;
        let mut atk = ((r.base_attack as f32) * 1.0786).round() as i32;
        let mut def = ((r.base_defense as f32) * 1.0942857).round() as i32;
        let mut mdef = ((r.base_mdefense as f32) * 1.0942857).round() as i32;
        let technic = ((r.base_technic as f32) * 1.395604).round() as i32;

        if let Some(equip) = equip {
            let game = configs::get();

            if let Some(s) = game
                .equip_strengthen
                .iter()
                .find(|s| s.strength_type == equip.equip_id)
            {
                hp += s.hp;
                atk += s.atk;
                def += s.def;
                mdef += s.mdef;
            }
        }

        HeroAttribute {
            hp: Some(hp),
            attack: Some(atk),
            defense: Some(def),
            mdefense: Some(mdef),
            technic: Some(technic),
            multi_hp_idx: Some(r.base_multi_hp_idx),
            multi_hp_num: Some(r.base_multi_hp_num),
        }
    }
}
