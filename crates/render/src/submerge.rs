//! A kart in mud or water is cut at the water line (`SPLIT_LINE`): the triangles that cross it
//! are split there, and the two halves are drawn apart (the one under water behind the surface,
//! `depthBiasSecondary`). The retail renderer does it per frame on the model's triangles
//! (`RenderBucket_WaterSplitInterpolateVertex`); so does this, on one mesh part.

use crate::mesh::MeshPart;

#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
    uv: [f32; 2],
}

impl Vertex {
    fn of(part: &MeshPart, i: u32) -> Vertex {
        let i = i as usize;
        Vertex {
            position: part.positions[i],
            color: part.colors[i],
            uv: part.uvs[i],
        }
    }

    fn lerp(self, to: Vertex, t: f32) -> Vertex {
        fn mix<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
            std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
        }
        Vertex {
            position: mix(self.position, to.position, t),
            color: mix(self.color, to.color, t),
            uv: mix(self.uv, to.uv, t),
        }
    }
}

/// The part of the triangle on one side of the line: a polygon of up to four corners.
fn clip(corners: [(Vertex, f32); 3], above: bool) -> Vec<Vertex> {
    let inside = |h: f32| (h >= 0.0) == above;
    let mut polygon = Vec::with_capacity(4);
    for i in 0..3 {
        let (from, from_height) = corners[i];
        let (to, to_height) = corners[(i + 1) % 3];
        if inside(from_height) {
            polygon.push(from);
        }
        if inside(from_height) != inside(to_height) {
            let t = from_height / (from_height - to_height);
            polygon.push(from.lerp(to, t));
        }
    }
    polygon
}

/// Splits `part` where `heights` (each vertex's height over the line) change sign: the faces over
/// the line (or on it) and the faces under it. The triangles keep their winding.
pub fn split(part: &MeshPart, heights: &[f32]) -> (MeshPart, MeshPart) {
    let (mut over, mut under) = (MeshPart::default(), MeshPart::default());
    for tri in part.indices.chunks_exact(3) {
        let corners = [0, 1, 2].map(|k| (Vertex::of(part, tri[k]), heights[tri[k] as usize]));
        for (half, above) in [(&mut over, true), (&mut under, false)] {
            let polygon = clip(corners, above);
            if polygon.len() < 3 {
                continue;
            }
            let first = half.positions.len() as u32;
            for v in &polygon {
                half.positions.push(v.position);
                half.colors.push(v.color);
                half.uvs.push(v.uv);
            }
            for k in 1..polygon.len() as u32 - 1 {
                half.indices.extend([first, first + k, first + k + 1]);
            }
        }
    }
    (over, under)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle(ys: [f32; 3]) -> (MeshPart, Vec<f32>) {
        let part = MeshPart {
            positions: ys
                .iter()
                .enumerate()
                .map(|(i, &y)| [i as f32, y, 0.0])
                .collect(),
            colors: vec![[1.0; 4]; 3],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 1, 2],
        };
        (part, ys.to_vec())
    }

    #[test]
    fn a_triangle_on_one_side_is_kept_whole() {
        let (part, heights) = triangle([1.0, 2.0, 3.0]);
        let (over, under) = split(&part, &heights);
        assert_eq!(over.indices.len(), 3);
        assert!(under.indices.is_empty());
        let (part, heights) = triangle([-1.0, -2.0, -3.0]);
        let (over, under) = split(&part, &heights);
        assert!(over.indices.is_empty());
        assert_eq!(under.indices.len(), 3);
    }

    #[test]
    fn a_triangle_across_the_line_is_cut_on_it() {
        // One corner over, two under: a triangle over, a quad (two triangles) under.
        let (part, heights) = triangle([2.0, -2.0, -2.0]);
        let (over, under) = split(&part, &heights);
        assert_eq!(over.indices.len(), 3);
        assert_eq!(under.indices.len(), 6);
        // Every new corner is on the line.
        let on_line = |half: &MeshPart| half.positions.iter().filter(|p| p[1] == 0.0).count();
        assert_eq!(on_line(&over), 2);
        assert_eq!(on_line(&under), 2);
        // And no corner is on the wrong side.
        assert!(over.positions.iter().all(|p| p[1] >= 0.0));
        assert!(under.positions.iter().all(|p| p[1] <= 0.0));
    }

    #[test]
    fn the_cut_corner_is_between_its_ends() {
        let (mut part, heights) = triangle([3.0, -1.0, -1.0]);
        part.uvs = vec![[0.0, 0.0], [1.0, 1.0], [1.0, 1.0]];
        part.colors[0] = [0.0, 0.0, 0.0, 1.0];
        let (over, _) = split(&part, &heights);
        // 3 -> -1 crosses at 3/4 of the way.
        let cut = over.positions.iter().position(|p| p[1] == 0.0).unwrap();
        assert!((over.uvs[cut][0] - 0.75).abs() < 1e-6);
        assert!((over.colors[cut][0] - 0.75).abs() < 1e-6);
    }
}
