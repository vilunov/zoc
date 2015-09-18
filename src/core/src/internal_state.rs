// See LICENSE file for copyright and license details.

use std::collections::{HashMap};
use cgmath::{Vector2};
use common::types::{PlayerId, UnitId, MapPos, Size2};
use core::{CoreEvent, FireMode, UnitInfo};
use unit::{Unit};
use db::{Db};
use map::{Map, Terrain};
use command::{MoveMode};

pub enum InfoLevel {
    Full,
    Partial,
}

pub struct InternalState {
    units: HashMap<UnitId, Unit>,
    map: Map<Terrain>,
}

impl<'a> InternalState {
    pub fn new(map_size: &Size2) -> InternalState {
        let mut map = Map::new(map_size, Terrain::Plain);
        // TODO: read from scenario.json?
        *map.tile_mut(&MapPos{v: Vector2{x: 4, y: 3}}) = Terrain::Trees;
        *map.tile_mut(&MapPos{v: Vector2{x: 4, y: 4}}) = Terrain::Trees;
        *map.tile_mut(&MapPos{v: Vector2{x: 4, y: 5}}) = Terrain::Trees;
        *map.tile_mut(&MapPos{v: Vector2{x: 5, y: 5}}) = Terrain::Trees;
        *map.tile_mut(&MapPos{v: Vector2{x: 6, y: 4}}) = Terrain::Trees;
        InternalState {
            units: HashMap::new(),
            map: map,
        }
    }

    pub fn units(&self) -> &HashMap<UnitId, Unit> {
        &self.units
    }

    pub fn unit(&'a self, id: &UnitId) -> &'a Unit {
        &self.units[id]
    }

    pub fn map(&'a self) -> &Map<Terrain> {
        &self.map
    }

    pub fn units_at(&'a self, pos: &MapPos) -> Vec<&'a Unit> {
        let mut units = Vec::new();
        for (_, unit) in &self.units {
            if unit.pos == *pos {
                units.push(unit);
            }
        }
        units
    }

    pub fn is_tile_occupied(&self, pos: &MapPos) -> bool {
        // TODO: optimize
        self.units_at(pos).len() > 0
    }

    /// Converts active ap (attack points) to reactive
    fn convert_ap(&mut self, player_id: &PlayerId) {
        for (_, unit) in self.units.iter_mut() {
            if unit.player_id == *player_id {
                if let Some(ref mut reactive_attack_points)
                    = unit.reactive_attack_points
                {
                    *reactive_attack_points += unit.attack_points;
                }
                unit.attack_points = 0;
            }
        }
    }

    fn refresh_units(&mut self, db: &Db, player_id: &PlayerId) {
        for (_, unit) in self.units.iter_mut() {
            if unit.player_id == *player_id {
                let unit_type = db.unit_type(&unit.type_id);
                unit.move_points = unit_type.move_points;
                unit.attack_points = unit_type.attack_points;
                if let Some(ref mut reactive_attack_points) = unit.reactive_attack_points {
                    *reactive_attack_points = unit_type.reactive_attack_points;
                }
                unit.morale += 10;
            }
        }
    }

    fn add_unit(&mut self, db: &Db, unit_info: &UnitInfo, info_level: InfoLevel) {
        assert!(self.units.get(&unit_info.unit_id).is_none());
        let unit_type = db.unit_type(&unit_info.type_id);
        self.units.insert(unit_info.unit_id.clone(), Unit {
            id: unit_info.unit_id.clone(),
            pos: unit_info.pos.clone(),
            player_id: unit_info.player_id.clone(),
            type_id: unit_info.type_id.clone(),
            move_points: unit_type.move_points,
            attack_points: unit_type.attack_points,
            reactive_attack_points: if let InfoLevel::Full = info_level {
                Some(unit_type.reactive_attack_points)
            } else {
                None
            },
            count: unit_type.count,
            morale: 100,
            passanger_id: if let InfoLevel::Full = info_level {
                unit_info.passanger_id.clone()
            } else {
                None
            },
        });
    }

    pub fn apply_event(&mut self, db: &Db, event: &CoreEvent) {
        match event {
            &CoreEvent::Move{ref unit_id, ref path, ref mode} => {
                let pos = path.destination().clone();
                let unit = self.units.get_mut(unit_id)
                    .expect("Bad move unit id");
                unit.pos = pos;
                assert!(unit.move_points > 0);
                if db.unit_type(&unit.type_id).is_transporter {
                    // TODO: get passanger and update its pos
                }
                if let &MoveMode::Fast = mode {
                    unit.move_points -= path.total_cost().n;
                } else {
                    unit.move_points -= path.total_cost().n * 2;
                }
                assert!(unit.move_points >= 0);
            },
            &CoreEvent::EndTurn{ref new_id, ref old_id} => {
                self.refresh_units(db, new_id);
                self.convert_ap(old_id);
            },
            &CoreEvent::CreateUnit{ref unit_info} => {
                self.add_unit(db, unit_info, InfoLevel::Full);
            },
            &CoreEvent::AttackUnit {
                ref attacker_id,
                ref defender_id,
                ref mode,
                ref killed,
                ref suppression,
                ref remove_move_points,
            } => {
                {
                    let unit = self.units.get_mut(defender_id)
                        .expect("Can`t find defender");
                    unit.count -= *killed;
                    unit.morale -= *suppression;
                    if *remove_move_points {
                        unit.move_points = 0;
                    }
                }
                let count = self.units[defender_id].count.clone();
                if count <= 0 {
                    // TODO: kill\unload passangers
                    assert!(self.units.get(defender_id).is_some());
                    self.units.remove(defender_id);
                }
                let attacker_id = match attacker_id.clone() {
                    Some(attacker_id) => attacker_id,
                    None => return,
                };
                if let Some(unit) = self.units.get_mut(&attacker_id) {
                    match mode {
                        &FireMode::Active => {
                            assert!(unit.attack_points >= 1);
                            unit.attack_points -= 1;
                        },
                        &FireMode::Reactive => {
                            if let Some(ref mut reactive_attack_points)
                                = unit.reactive_attack_points
                            {
                                assert!(*reactive_attack_points >= 1);
                                *reactive_attack_points -= 1;
                            }
                        },
                    }
                }
            },
            &CoreEvent::ShowUnit{ref unit_info} => {
                self.add_unit(db, unit_info, InfoLevel::Partial);
            },
            &CoreEvent::HideUnit{ref unit_id} => {
                assert!(self.units.get(unit_id).is_some());
                self.units.remove(unit_id);
            },
            &CoreEvent::LoadUnit{ref passanger_id, ref transporter_id} => {
                // TODO: hide info abiut passanger from enemy player
                self.units.get_mut(transporter_id)
                    .expect("Bad transporter_id")
                    .passanger_id = Some(passanger_id.clone());
                let transporter_pos = self.units[transporter_id].pos.clone();
                let passanger = self.units.get_mut(passanger_id)
                    .expect("Bad passanger_id");
                passanger.pos = transporter_pos;
                passanger.move_points = 0;
            },
            &CoreEvent::UnloadUnit{ref transporter_id, ref unit_info} => {
                self.units.get_mut(transporter_id)
                    .expect("Bad transporter_id")
                    .passanger_id = None;
                if let Some(unit) = self.units.get_mut(&unit_info.unit_id) {
                    unit.pos = unit_info.pos.clone();
                    return;
                }
                self.add_unit(db, unit_info, InfoLevel::Partial);
            },
        }
    }
}

// vim: set tabstop=4 shiftwidth=4 softtabstop=4 expandtab:
