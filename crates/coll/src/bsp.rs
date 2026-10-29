//! The walk of the BSP tree that finds the leaves a search touches.

use loader::lev::{BspChild, BspKind, BspNode, Level};

use crate::BoundingBox;
use crate::types::BSP_STACK_LEN;

/// Do two bounding boxes touch (the BSP search's test; inclusive).
fn overlaps(node: &BspNode, bounds: &BoundingBox) -> bool {
    let b = &node.bbox;
    b.min.y <= bounds.max.y
        && b.min.x <= bounds.max.x
        && bounds.min.x <= b.max.x
        && b.min.z <= bounds.max.z
        && bounds.min.z <= b.max.z
        && bounds.min.y <= b.max.y
}


/// `COLL_SearchBSP_CallbackPARAM`: calls `callback` on every BSP leaf whose
/// bounds touch `bounds`, in the reference's order (the second child of a
/// branch is visited before the first).
pub(crate) fn search_bsp<T>(
    level: &Level,
    bounds: &BoundingBox,
    callback: impl Fn(&mut T, &Level, &BspNode),
    state: &mut T,
) {
    let Some(root) = level.bsp.first() else {
        return;
    };
    let mut stack = [BspChild::None; BSP_STACK_LEN];
    let mut top = 0;

    let push_children = |node: &BspNode, stack: &mut [BspChild; BSP_STACK_LEN], top: &mut usize| {
        let BspKind::Branch { children, .. } = &node.kind else {
            return;
        };
        for &child in &children[..2] {
            let index = match child {
                BspChild::None => continue,
                BspChild::Branch(i) | BspChild::Leaf(i) => i,
            };
            if overlaps(&level.bsp[index as usize], bounds) {
                stack[*top] = child;
                *top += 1;
            }
        }
    };

    push_children(root, &mut stack, &mut top);
    while top > 0 {
        top -= 1;
        match stack[top] {
            BspChild::None => unreachable!(),
            BspChild::Leaf(i) => callback(state, level, &level.bsp[i as usize]),
            BspChild::Branch(i) => push_children(&level.bsp[i as usize], &mut stack, &mut top),
        }
    }
}
