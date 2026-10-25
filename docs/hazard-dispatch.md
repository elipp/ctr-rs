# Hazard dispatch: typed lists or fully dynamic objects

Status: undecided. The first step (below, "Where we are") is done; the second ("Option B") is a sketch that has not been
compiled or tried.

## The problem

Every kind of object a level places (armadillo, fireball, minecart, roller, turtle, fruit, crate) used to be listed by
hand in about ten places, so adding the next kind (the weapon projectiles are the obvious candidates) meant touching all of
them:

- `Objects` had a `Vec<Placed<T>>` field per kind (`crates/instance/src/lib.rs`).
- `Objects::new` had a `match` on the model name with one `push` per kind.
- `Objects::tick` called `th_tick_all` once per kind.
- `Instances::touch` was a chain of `.or_else(touch_in(..))`, one per kind.
- `view/props.rs` had a `Hazard` enum, seven `by_instance.extend(..)` blocks to fill it, and a `match` on it that spawns
  each kind's body entity.

## Where we are (Option A: typed lists, dynamic walk)

The lists stay typed. The sim reaches them through the `Instance` trait that already existed.

- `Objects::each_object(f)` walks every object as `Entry { instance, object: &mut dyn Instance, cx, random_number }`,
  in the order the threads run. `tick` and `touch` both use it.
- `Objects::slots()` yields `(instance, Slot)` for every object. `Slot` is `Armadillo(i)`, `Crate(i)` and so on, and
  replaced the view's private `Hazard` enum.
- Adding a kind: a field on `Objects`, an arm in `new`, one line in `each_object`, one line in `slots`, plus what the view
  does for it.

Diff size was flat (+108 / -107). The gain is that the kinds are listed in two places in `instance` instead of about
ten, not that there is less code.

Tick order is kept exactly: armadillos, fireballs, minecarts, turtles, fruit, crates, rollers. The sim and the desync
hashes depend on it.

Known awkward bits: `Entry` carries `random_number` only because `touch` needs it while the objects are borrowed, and
`each_object` cannot stop early, so `touch` scans every object (it was a linear `find` per kind before, so no worse).

## Option B: fully dynamic objects (not done)

One list of boxed objects, and the kinds register themselves.

### The trait, with the clone and equality code written once

The sim state has to stay `Clone + PartialEq + Debug` for rollback snapshots and the desync hash. A blanket impl gives
that to any kind that already derives them (all of them do):

```rust
pub trait Hazard: Instance + Debug + Send + Sync + 'static {
    fn box_clone(&self) -> Box<dyn Hazard>;
    fn as_any(&self) -> &dyn Any;
    fn eq_dyn(&self, other: &dyn Hazard) -> bool;
}

impl<T: Instance + Clone + PartialEq + Debug + Send + Sync + 'static> Hazard for T {
    fn box_clone(&self) -> Box<dyn Hazard> { Box::new(self.clone()) }
    fn as_any(&self) -> &dyn Any { self }
    fn eq_dyn(&self, other: &dyn Hazard) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }
}
impl Clone for Box<dyn Hazard> { fn clone(&self) -> Self { self.box_clone() } }
impl PartialEq for Box<dyn Hazard> { fn eq(&self, o: &Self) -> bool { self.eq_dyn(&**o) } }
```

### `Objects` loses its seven lists

```rust
pub struct Objects {
    placed: Vec<Placed<Box<dyn Hazard>>>,   // in the level's instance order
    by_instance: HashMap<usize, usize>,     // instance -> index into `placed`
    pub random_number: i32,
    position_path: Vec<SVec3>,
    roll_paths: Vec<Vec<PosRot>>,
    ticks: u32,
}

impl Objects {
    pub fn tick(&mut self) -> Tick { /* for p in &mut self.placed { p.sim.th_tick(..) } */ }
    pub fn touch(..)  { /* placed[by_instance[&instance]].sim.touch(..): O(1) */ }

    /// Typed access for the view: `objects.get::<CrateBox>(i).scale`
    pub fn get<T: Hazard>(&self, i: usize) -> &T {
        self.placed[i].sim.as_any().downcast_ref().expect("the kind it was placed as")
    }
}
```

`each_object`, `Entry`, `Slot` and `slots_of` go away.

### `new` becomes a table

```rust
type Build = fn(&InstDef, &PlaceCx) -> Option<Box<dyn Hazard>>;
const KINDS: &[(fn(&InstDef) -> bool, Build)] = &[
    (|d| Kind::of_model(d.model_id).is_some(), |d, _| Some(Box::new(CrateBox::new(Kind::of_model(d.model_id)?)))),
    (|d| d.model_name == "armadillo", |d, cx| Some(Box::new(Armadillo::new(d, cx.delay(d))))),
    (|d| d.model_name == "fruit",     |_, _|  Some(Box::new(Fruit::default()))),
    (|d| d.model_name == "minecart",  |d, cx| Minecart::new(d, &cx.position_path).map(|m| Box::new(m) as _)),
    // ...
];
```

A new kind is one row. (Crates are picked by `model_id`, the rest by `model_name`, hence the matcher function.)

### The view

Only 8 places outside `instance` read the concrete lists: `props.rs` (crates, fruit, turtles, armadillos, minecarts,
and two `.len()` log lines), `explosion.rs` (crates). They become `get::<T>(i)`.

The view-side `match` in `props::spawn` could go too, with a `HashMap<TypeId, fn(&mut BodyCx, usize) -> Option<Entity>>`
that each kind registers into, and the per-kind pose systems (`pose_hazards`, `hide_fruit`, `pose_crates`) turned into
small per-kind plugins. That part is optional and can be done separately from the sim side.

## Comparison

|                    | A: typed lists + dyn walk (now)           | B: fully dynamic                           |
|--------------------|-------------------------------------------|--------------------------------------------|
| Add a kind         | field, `new` arm, 2 list lines, view arm  | one `KINDS` row, one view registration     |
| View reads         | plain indexing, checked at compile time   | `get::<T>(i)` downcast, panics if wrong    |
| `touch`            | scans all objects                         | O(1) by instance                           |
| Tick order         | grouped by kind                           | level instance order                       |
| Rollback clone     | free                                      | one heap allocation per object per snapshot |
| Net lines          | about flat                                | about -150                                 |

## Risks of B

- **Tick order changes.** From grouped by kind to level instance order. That may be closer to retail, whose threads run
  in creation order, but it changes every hash and any recorded demo. Check against the C oracle on the Linux box before
  trusting it.
- **Rollback cost.** Snapshots clone `Objects` every tick over a 120-tick window. Boxing turns each object's clone into
  an allocation, with hundreds of objects on a big track. Probably fine, but measure `Rollback` before and after.
- **Downcast panics.** A view body that holds the index of the wrong kind panics at runtime. Option A cannot get that
  wrong.

## Recommendation

Stay on A for the current seven kinds. Take B if the projectile weapons (bombs, missiles, TNT, potions, the warp ball)
land as placed objects, since that is where the kind list will grow. It is a contained change: `crates/instance/src/lib.rs`
and a handful of lines in `crates/view/src/props.rs` and `explosion.rs`. Do it on a branch, run the `instance`, `race`
and `view` tests plus the oracle, and compare `Rollback` timing.
