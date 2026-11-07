//! f64 diagnostic traversal, not a prediction of future GPU arithmetic or cost.
use super::*;
/// Width (diameter) and linear growth per unit ray parameter, in object space.
/// A caller transforming a world ray must transform its cone conservatively too.
#[derive(Clone, Copy, Debug)]
pub struct Cone {
    pub width: f64,
    pub spread: f64,
}
impl Cone {
    pub fn valid(self) -> bool {
        self.width.is_finite() && self.spread.is_finite() && self.width >= 0. && self.spread >= 0.
    }
    pub fn width_at(self, t: f64) -> f64 {
        self.width + self.spread * t
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
    pub t_min: f64,
    pub t_max: f64,
}
impl Ray {
    pub fn valid(self) -> bool {
        self.origin
            .iter()
            .chain(&self.direction)
            .all(|v| v.is_finite())
            && self.direction.iter().any(|&v| v != 0.)
            && self.t_min.is_finite()
            && !self.t_max.is_nan()
            && self.t_min >= 0.
            && self.t_max >= self.t_min
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Ray parameter t; a distance when direction is unit length.
    pub distance: f64,
    pub leaf: u32,
    pub triangle: u32,
    pub barycentrics: [f64; 2],
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn box_hit(ray: Ray, lo: [f64; 3], hi: [f64; 3], max: f64) -> bool {
    let mut near = ray.t_min;
    let mut far = max;
    for a in 0..3 {
        if ray.direction[a] == 0. {
            if ray.origin[a] < lo[a] || ray.origin[a] > hi[a] {
                return false;
            }
        } else {
            let x = (lo[a] - ray.origin[a]) / ray.direction[a];
            let y = (hi[a] - ray.origin[a]) / ray.direction[a];
            near = near.max(x.min(y));
            far = far.min(x.max(y));
            if near > far {
                return false;
            }
        }
    }
    true
}
fn box_distance(p: [f64; 3], lo: [f64; 3], hi: [f64; 3]) -> f64 {
    (0..3)
        .map(|a| (lo[a] - p[a]).max(0.).max(p[a] - hi[a]).powi(2))
        .sum()
}
fn point_triangle(p: [f64; 3], [a, b, c]: [[f64; 3]; 3]) -> f64 {
    let mut best = f64::INFINITY;
    for (a, b) in [(a, b), (b, c), (c, a)] {
        let edge = sub(b, a);
        let length = dot(edge, edge);
        let t = if length == 0. {
            0.
        } else {
            (dot(sub(p, a), edge) / length).clamp(0., 1.)
        };
        let q = std::array::from_fn(|i| a[i] + edge[i] * t);
        best = best.min(dot(sub(p, q), sub(p, q)));
    }
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let normal = cross(ab, ac);
    let n2 = dot(normal, normal);
    if n2 > 0. {
        let v = dot(cross(ap, ac), normal) / n2;
        let w = dot(cross(ab, ap), normal) / n2;
        if v >= 0. && w >= 0. && v + w <= 1. {
            best = best.min(dot(ap, normal).powi(2) / n2);
        }
    }
    best
}
impl Bvh {
    pub fn triangle(&self, leaf: u32, triangle: u32) -> [[f64; 3]; 3] {
        let l = &self.leaves[leaf as usize];
        std::array::from_fn(|i| {
            let index =
                self.indices[l.indices.offset as usize + triangle as usize * 3 + i] as usize;
            std::array::from_fn(|a| f64::from(self.positions[index][a]))
        })
    }
    fn intersect_leaf(&self, leaf: u32, ray: Ray, best: &mut Option<Hit>) {
        let l = &self.leaves[leaf as usize];
        for triangle in 0..l.indices.count / 3 {
            let [a, b, c] = self.triangle(leaf, triangle);
            let ab = sub(b, a);
            let ac = sub(c, a);
            let p = cross(ray.direction, ac);
            let det = dot(ab, p);
            if det == 0. {
                continue;
            }
            let s = sub(ray.origin, a);
            let u = dot(s, p) / det;
            if !(0. ..=1.).contains(&u) {
                continue;
            }
            let q = cross(s, ab);
            let v = dot(ray.direction, q) / det;
            if v < 0. || u + v > 1. {
                continue;
            }
            let t = dot(ac, q) / det;
            let hit = Hit {
                distance: t,
                leaf,
                triangle,
                barycentrics: [u, v],
            };
            if t >= ray.t_min
                && t <= ray.t_max
                && best.is_none_or(|b| {
                    t < b.distance || (t == b.distance && (leaf, triangle) < (b.leaf, b.triangle))
                })
            {
                *best = Some(hit);
            }
        }
    }
    /// The caller validates a cooked BVH once; queries check only ray and level.
    pub fn trace(&self, level: usize, ray: Ray) -> Option<Hit> {
        if !ray.valid() {
            return None;
        }
        let mut stack = vec![self.levels.get(level)?.root];
        let mut best = None;
        while let Some(id) = stack.pop() {
            if id & LEAF_BIT != 0 {
                self.intersect_leaf(id & !LEAF_BIT, ray, &mut best);
            } else {
                let n = &self.nodes[id as usize];
                for slot in (0..4).rev() {
                    if n.children[slot] == EMPTY {
                        continue;
                    }
                    let (lo, hi) = n.child_bounds(slot);
                    if box_hit(ray, lo, hi, best.map_or(ray.t_max, |h: Hit| h.distance)) {
                        stack.push(n.children[slot]);
                    }
                }
            }
        }
        best
    }
    pub fn trace_brute(&self, level: usize, ray: Ray) -> Option<Hit> {
        if !ray.valid() {
            return None;
        }
        let l = self.levels.get(level)?;
        let mut best = None;
        for &leaf in &self.level_leaves[l.leaves.range(self.level_leaves.len()).ok()?] {
            self.intersect_leaf(leaf, ray, &mut best);
        }
        best
    }
    /// Exact nearest triangle distance up to f64 arithmetic; used to measure
    /// conservative cluster-ball surface-distance bounds, not ray-distance bounds.
    pub fn nearest_distance(&self, level: usize, point: [f64; 3]) -> Option<f64> {
        if !point.iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut stack = vec![(self.levels.get(level)?.root, 0.)];
        let mut best = f64::INFINITY;
        while let Some((id, distance)) = stack.pop() {
            if distance > best {
                continue;
            }
            if id & LEAF_BIT != 0 {
                let leaf = id & !LEAF_BIT;
                for t in 0..self.leaves[leaf as usize].indices.count / 3 {
                    best = best.min(point_triangle(point, self.triangle(leaf, t)));
                }
            } else {
                let n = &self.nodes[id as usize];
                let mut children = [(EMPTY, f64::INFINITY); 4];
                for (slot, entry) in children.iter_mut().enumerate() {
                    if n.children[slot] == EMPTY {
                        continue;
                    }
                    let (lo, hi) = n.child_bounds(slot);
                    *entry = (n.children[slot], box_distance(point, lo, hi));
                }
                children.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
                stack.extend(
                    children
                        .into_iter()
                        .filter(|&(id, d)| id != EMPTY && d <= best),
                );
            }
        }
        Some(best.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nearest_triangle_handles_face_edge_vertex_and_degenerate_geometry() {
        let triangle = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]];
        for (point, expected_squared) in [
            ([0.5, 0.5, 3.], 9.),
            ([1., -2., 0.], 4.),
            ([-1., -1., 0.], 2.),
            ([2., 2., 0.], 2.),
        ] {
            assert!((point_triangle(point, triangle) - expected_squared).abs() < 1e-12);
        }
        assert_eq!(point_triangle([1., 2., 3.], [[0.; 3]; 3]), 14.);
        assert_eq!(
            point_triangle([1., 2., 0.], [[0.; 3], [2., 0., 0.], [1., 0., 0.]]),
            4.
        );
    }
}
