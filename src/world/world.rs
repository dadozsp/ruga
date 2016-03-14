use super::spatial_hashing::{SpatialHashing, Identifiable};
use super::{EntityCell, Entity};
use super::body::{PhysicType, Flags, Location};
use super::utils::grid_raycast;
use super::utils::minus_pi_pi;
use super::FrameManager;
use super::EffectManager;

use std::rc::Rc;
use std::collections::HashSet;
use std::cmp::Ordering;

pub struct World {
    pub unit: f64,
    pub time: f64,
    next_id: usize,

    wall_map: HashSet<(i32,i32)>,
    entity_cells: Vec<Rc<EntityCell>>,
    static_hashmap: SpatialHashing<Rc<EntityCell>>,
    dynamic_hashmap: SpatialHashing<Rc<EntityCell>>,
}

impl Identifiable for Rc<EntityCell> {
    fn id(&self) -> usize {
        self.borrow().body().id
    }
}

impl World {
    pub fn new(unit: f64) -> World {
        World {
            unit: unit,
            time: 0.,
            next_id: 1,
            wall_map: HashSet::new(),
            entity_cells: Vec::new(),
            static_hashmap: SpatialHashing::new(unit),
            dynamic_hashmap: SpatialHashing::new(unit),
        }
    }

    pub fn unit(&self) -> f64 {
        self.unit
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn render(&mut self, frame_manager: &mut FrameManager) {
        for entity_cell in &self.entity_cells {
            entity_cell.borrow().render(frame_manager);
        }
    }

    pub fn update(&mut self, dt: f64, effect_manager: &mut EffectManager) {
        for entity_cell in &self.entity_cells {
            entity_cell.update(dt,&self,effect_manager);
        }

        let mut i = 0;
        while i < self.entity_cells.len() {
            let b = self.entity_cells[i].borrow().body().dead();
            if b {
                self.entity_cells.swap_remove(i);
            } else {
                i += 1;
            }
        }

        self.clear_dynamic();
        for entity_cell in &self.entity_cells {
            {
                let entity = &mut *entity_cell.borrow_mut();
                let location = entity.body().location();
                let mask = entity.body().mask;
                let mut callback = |other: &mut Entity| {
                    if entity.body().collide(other.body()) {
                        entity.mut_body().resolve_collision(other.body());
                        other.mut_body().resolve_collision(entity.body());
                        entity.on_collision(other);
                        other.on_collision(entity);
                    }
                };
                self.apply_locally(mask,&location,&mut callback);
            }
            self.dynamic_hashmap.insert_locally(&entity_cell.borrow().body().location(),entity_cell);
        }
    }

    pub fn entity_cells(&self) -> &Vec<Rc<EntityCell>> {
        &self.entity_cells
    }

    pub fn insert(&mut self, entity: &Rc<EntityCell>) {
        entity.borrow_mut().mut_body().id = self.next_id;
        self.next_id += 1;

        entity.borrow().modify_wall_map(&mut self.wall_map);

        match entity.borrow().body().physic_type {
            PhysicType::Static => self.static_hashmap.insert_locally(&entity.borrow().body().location(),entity),
            _ => self.dynamic_hashmap.insert_locally(&entity.borrow().body().location(),entity),
        }
        self.entity_cells.push(entity.clone());
    }

    pub fn apply_on_group<F: FnMut(&mut Entity)>(&self, mask: Flags, callback: &mut F) {
        for entity_cell in &self.entity_cells {
            let mut entity = entity_cell.borrow_mut();
            if entity.body().group & mask != 0 {
                callback(&mut *entity);
            }
        }
    }

    pub fn apply_locally<F: FnMut(&mut Entity)>(&self, mask: Flags, loc: &Location, callback: &mut F) {
        self.static_hashmap.apply_locally(loc, &mut |entity_cell: &Rc<EntityCell>| {
            let mut entity = entity_cell.borrow_mut();
            if (entity.body().group & mask != 0) && entity.body().in_location(loc) {
                callback(&mut *entity);
            }
        });
        self.dynamic_hashmap.apply_locally(loc, &mut |entity_cell: &Rc<EntityCell>| {
            let mut entity = entity_cell.borrow_mut();
            if (entity.body().group & mask != 0) && entity.body().in_location(loc) {
                callback(&mut *entity);
            }
        });
    }

    pub fn apply_on_index<F: FnMut(&mut Entity)>(&self, mask: Flags, index: &[i32;2], callback: &mut F) {
        let c = &mut |entity_cell: &Rc<EntityCell>| {
            let mut entity = entity_cell.borrow_mut();
            if entity.body().group & mask != 0 {
                callback(&mut *entity);
            }
        };
        self.static_hashmap.apply_on_index(index,c);
        self.dynamic_hashmap.apply_on_index(index,c);
    }

    /// callback return true when stop
    pub fn raycast<F: FnMut(&mut Entity, f64, f64) -> bool>(&self, mask: Flags, x: f64, y: f64, angle: f64, length: f64, callback: &mut F) {
        use std::f64::consts::PI;

        //println!("");
        //println!("raycast");

        let angle = minus_pi_pi(angle);

        let unit = self.static_hashmap.unit();
        let x0 = x;
        let y0 = y;
        let x1 = x+length*angle.cos();
        let y1 = y+length*angle.sin();
        let index_vec = grid_raycast(x0/unit, y0/unit, x1/unit, y1/unit);

        // equation ax + by + c = 0
        let (a,b,c) = if angle.abs() == PI || angle == 0. {
            (0.,1.,-y)
        } else {
            let b = -1./angle.tan();
            (1.,b,-x-b*y)
        };

        let line_start = x0.min(x1);
        let line_end = x0.max(x1);

        let mut bodies: Vec<(Rc<EntityCell>,f64,f64)>;
        let mut visited = HashSet::new();
        for i in &index_vec {
            //println!("index:{:?}",i);
            // abscisse of start and end the segment of
            // the line that is in the current square
            let segment_start = ((i[0] as f64)*unit).max(line_start);
            let segment_end = (((i[0]+1) as f64)*unit).min(line_end);

            bodies = Vec::new();

            let mut res = self.static_hashmap.get_on_index(i);
            res.append(&mut self.dynamic_hashmap.get_on_index(i));
            res.retain(|entity| {
                (entity.borrow().body().group & mask != 0) && !visited.contains(&entity.borrow().body().id)
            });
            while let Some(entity) = res.pop() {
                let intersections = entity.borrow().body().raycast(a,b,c);
                if let Some((x_min,y_min,x_max,y_max)) = intersections {
                    //println!("intersection");
                    //println!("start:{},end:{},min:{},max:{}",segment_start,segment_end,x_min,x_max);

                    if angle.abs() > PI/2. {
                        if segment_start <= x_max && x_min <= segment_end {
                            visited.insert(entity.borrow().body().id);
                            //println!("intersection in segment");
                            let max = ((x0-x_min).powi(2) + (y0-y_min).powi(2)).sqrt();
                            let mut min = ((x0-x_max).powi(2) + (y0-y_max).powi(2)).sqrt();
                            if x_max > segment_end {
                                min = -min;
                            }
                            bodies.push((entity,min,max));
                        }
                    } else {
                        if segment_start <= x_max && x_min <= segment_end {
                            visited.insert(entity.borrow().body().id);
                            //println!("intersection in segment");
                            let mut min = ((x0-x_min).powi(2) + (y0-y_min).powi(2)).sqrt();
                            let max = ((x0-x_max).powi(2) + (y0-y_max).powi(2)).sqrt();
                            if x_min < segment_start {
                                min = -min;
                            }
                            bodies.push((entity,min,max));
                        }
                    }
                }
            }

            bodies.sort_by(|&(_,min_a,_),&(_,min_b,_)| {
                if min_a > min_b {
                    Ordering::Greater
                } else if min_a == min_b {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            });

            for (entity,min,max) in bodies {
                visited.insert(entity.borrow().body().id);
                if callback(&mut *entity.borrow_mut(),min,max) {
                    return;
                }
            }
        }
    }

    pub fn get_on_segment<F: FnMut(&mut EntityCell, f64, f64) -> bool>(&self, _mask: Flags, _x: f64, _y: f64, _angle: f64, _length: f64, _callback: &mut F) {
        assert!(false);
    }

    pub fn get_on_index(&self, mask: Flags, index: &[i32;2]) -> Vec<Rc<EntityCell>> {
        let mut vec = Vec::new();
        vec.append(&mut self.static_hashmap.get_on_index(index));
        vec.append(&mut self.dynamic_hashmap.get_on_index(index));
        vec.retain(&mut |entity: &Rc<EntityCell>| {
            entity.borrow().body().group & mask != 0
        });
        vec
    }

    pub fn get_locally(&self, mask: Flags, loc: &Location) -> Vec<Rc<EntityCell>> {
        let mut vec = Vec::new();
        vec.append(&mut self.static_hashmap.get_locally(loc));
        vec.append(&mut self.dynamic_hashmap.get_locally(loc));
        vec.retain(&mut |entity: &Rc<EntityCell>| {
            let entity = entity.borrow();
            let entity = entity.body();
            (entity.group & mask != 0) && (entity.in_location(loc))
        });
        vec
    }

    pub fn get_on_group(&self, mask: Flags) -> Vec<Rc<EntityCell>> {
        let mut vec = Vec::new();
        for entity in &self.entity_cells {
            if entity.borrow().body().group & mask != 0 {
                vec.push(entity.clone());
            }
        }
        vec
    }

    fn clear_dynamic(&mut self) {
        self.dynamic_hashmap.clear();
    }
}

