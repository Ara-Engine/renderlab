//! TP1 alpha-tested (cutout) geometry, docs/m7-tp1.md. A mesh is cut to its alpha
//! mask's contour once, at load time. Every visibility, shadow and GI path then sees
//! foliage as ordinary opaque triangles, with no any-hit or alpha-test shader stage:
//! - raster and visibility-buffer primaries;
//! - traced primaries;
//! - Virtual, Surface, Interactive and Exact shadows;
//! - world radiance cache rays.
//!
//! Each source triangle is cut cell by cell on a uv grid of `cells` per uv unit. Marching
//! squares runs on the bilinear alpha field, with saddles decided by the cell centre, and
//! the cell polygon is clipped to the triangle in uv. Vertices interpolate the source
//! triangle's attributes; winding follows the source triangle.
use crate::{MeshData, Vertex};

/// A coverage mask over uv, sampled bilinearly with repeat wrapping and texel centres at
/// `(i + 0.5) / width`, as the base-colour texture is.
pub struct AlphaMask {
    pub width: u32,
    pub height: u32,
    /// Row-major alpha, one value per texel.
    pub alpha: Vec<f32>,
}
impl AlphaMask {
    /// The alpha channel of RGBA8 texels (a base-colour texture's finest level).
    pub fn from_rgba8(width: u32, height: u32, rgba8: &[u8]) -> Self {
        assert_eq!(rgba8.len(), width as usize * height as usize * 4);
        Self {
            width,
            height,
            alpha: rgba8.chunks_exact(4).map(|p| f32::from(p[3]) / 255.0).collect(),
        }
    }
    fn texel(&self, x: i64, y: i64) -> f32 {
        let (w, h) = (i64::from(self.width), i64::from(self.height));
        self.alpha[(y.rem_euclid(h) * w + x.rem_euclid(w)) as usize]
    }
    /// Bilinear alpha at `uv` with repeat wrapping.
    pub fn sample(&self, uv: [f32; 2]) -> f32 {
        let x = f64::from(uv[0]) * f64::from(self.width) - 0.5;
        let y = f64::from(uv[1]) * f64::from(self.height) - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.texel(x0, y0) * (1.0 - fx) + self.texel(x0 + 1, y0) * fx;
        let bottom = self.texel(x0, y0 + 1) * (1.0 - fx) + self.texel(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// An owned cut mesh; upload it with `MeshData` like any other.
#[derive(Default)]
pub struct CutoutMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}
impl CutoutMesh {
    pub fn data(&self) -> MeshData<'_> {
        MeshData {
            vertices: &self.vertices,
            indices: &self.indices,
        }
    }
}

/// Grid cells one source triangle may cover; larger uv footprints are rejected.
pub const MAX_CUTOUT_CELLS: u64 = 1 << 22;

/// Cuts `data` to where `opacity * mask(uv) * vertex alpha >= cutoff`, on a grid of `cells`
/// per uv unit. Typical foliage cards use 32-128 cells; the contour is linear within a cell.
/// Triangles with a degenerate uv mapping are kept or dropped whole, by their centroid.
pub fn cutout(
    data: &MeshData<'_>,
    mask: &AlphaMask,
    opacity: f32,
    cutoff: f32,
    cells: u32,
) -> Result<CutoutMesh, crate::Error> {
    if mask.width == 0
        || mask.height == 0
        || mask.alpha.len() != mask.width as usize * mask.height as usize
        || cells == 0
        || !(opacity.is_finite() && cutoff.is_finite())
        || data.indices.len() % 3 != 0
        || data.indices.iter().any(|&i| i as usize >= data.vertices.len())
    {
        return Err(crate::Error::InvalidInput(
            "cutout needs a nonempty mask, cells > 0, finite factors and valid triangles",
        ));
    }
    let mut out = CutoutMesh::default();
    let grid = f64::from(cells);
    for triangle in data.indices.chunks_exact(3) {
        let v = [0, 1, 2].map(|k| data.vertices[triangle[k] as usize]);
        let uv = v.map(|v| [f64::from(v.uv[0]), f64::from(v.uv[1])]);
        let field = |p: [f64; 2]| {
            let w = barycentric(&uv, p).unwrap_or([1.0 / 3.0; 3]);
            let alpha: f64 = (0..3).map(|k| w[k] * f64::from(v[k].color[3])).sum();
            f64::from(opacity * mask.sample([p[0] as f32, p[1] as f32])) * alpha - f64::from(cutoff)
        };
        let area = cross(sub(uv[1], uv[0]), sub(uv[2], uv[0]));
        if area.abs() < 1e-14 {
            let centre = [0, 1].map(|a| (uv[0][a] + uv[1][a] + uv[2][a]) / 3.0);
            if field(centre) >= 0.0 {
                push_polygon(&mut out, &v, &uv, &[uv[0], uv[1], uv[2]], area >= 0.0);
            }
            continue;
        }
        let lo = [0, 1].map(|a| (uv[0][a].min(uv[1][a]).min(uv[2][a]) * grid).floor() as i64);
        let hi = [0, 1].map(|a| (uv[0][a].max(uv[1][a]).max(uv[2][a]) * grid).ceil() as i64);
        let count = ((hi[0] - lo[0]).max(1) as u64).saturating_mul((hi[1] - lo[1]).max(1) as u64);
        if count > MAX_CUTOUT_CELLS {
            return Err(crate::Error::InvalidInput(
                "cutout triangle covers more than MAX_CUTOUT_CELLS grid cells",
            ));
        }
        // Corner values of one grid row, shared by its cells so neighbours agree exactly.
        let node = |i: i64, j: i64| [i as f64 / grid, j as f64 / grid];
        let mut below: Vec<f64> = (lo[0]..=hi[0]).map(|i| field(node(i, lo[1]))).collect();
        for j in lo[1]..hi[1] {
            let above: Vec<f64> = (lo[0]..=hi[0]).map(|i| field(node(i, j + 1))).collect();
            for i in lo[0]..hi[0] {
                let k = (i - lo[0]) as usize;
                let corners = [node(i, j), node(i + 1, j), node(i + 1, j + 1), node(i, j + 1)];
                let values = [below[k], below[k + 1], above[k + 1], above[k]];
                for polygon in cell_polygons(corners, values, || {
                    field([(i as f64 + 0.5) / grid, (j as f64 + 0.5) / grid])
                }) {
                    let clipped = clip_to_triangle(polygon, &uv, area > 0.0);
                    if clipped.len() >= 3 {
                        push_polygon(&mut out, &v, &uv, &clipped, area > 0.0);
                    }
                }
            }
            below = above;
        }
    }
    let extent = data
        .vertices
        .iter()
        .flat_map(|v| v.position)
        .fold(1e-6f32, |m, c| m.max(c.abs()));
    Ok(weld(out, extent))
}

/// Joins vertices that share a position and uv (cells emit their own; neighbours compute
/// the same points), then drops triangles that collapse or have no area, so the result is
/// a connected mesh: receiver adjacency then finds its edges.
fn weld(mesh: CutoutMesh, extent: f32) -> CutoutMesh {
    let scale = f64::from((1u32 << 22) as f32 / extent);
    let key = |v: &Vertex| -> [i64; 5] {
        let p = v.position.map(|c| (f64::from(c) * scale).round() as i64);
        let t = v.uv.map(|c| (f64::from(c) * f64::from(1u32 << 22)).round() as i64);
        [p[0], p[1], p[2], t[0], t[1]]
    };
    let mut slots = std::collections::HashMap::with_capacity(mesh.vertices.len());
    let mut out = CutoutMesh::default();
    let remap: Vec<u32> = mesh
        .vertices
        .iter()
        .map(|v| {
            *slots.entry(key(v)).or_insert_with(|| {
                out.vertices.push(*v);
                out.vertices.len() as u32 - 1
            })
        })
        .collect();
    let floor = (f64::from(extent) * 1e-7).powi(2);
    for t in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [0, 1, 2].map(|k| remap[t[k] as usize]);
        if a == b || b == c || a == c {
            continue;
        }
        let [pa, pb, pc] = [a, b, c].map(|i| out.vertices[i as usize].position.map(f64::from));
        let (e1, e2) = ([0, 1, 2].map(|k| pb[k] - pa[k]), [0, 1, 2].map(|k| pc[k] - pa[k]));
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        if n.iter().map(|x| x * x).sum::<f64>() * 0.25 <= floor {
            continue;
        }
        out.indices.extend([a, b, c]);
    }
    out
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn barycentric(uv: &[[f64; 2]; 3], p: [f64; 2]) -> Option<[f64; 3]> {
    let area = cross(sub(uv[1], uv[0]), sub(uv[2], uv[0]));
    if area.abs() < 1e-14 {
        return None;
    }
    let b = cross(sub(p, uv[0]), sub(uv[2], uv[0])) / area;
    let c = cross(sub(uv[1], uv[0]), sub(p, uv[0])) / area;
    Some([1.0 - b - c, b, c])
}

/// The inside (value >= 0) region of one cell as counter-clockwise uv polygons.
/// Corners are counter-clockwise from (i, j). A saddle is one polygon when the centre
/// is inside, two corner triangles otherwise.
fn cell_polygons(
    corners: [[f64; 2]; 4],
    values: [f64; 4],
    centre: impl FnOnce() -> f64,
) -> Vec<Vec<[f64; 2]>> {
    let inside = values.map(|v| v >= 0.0);
    if inside.iter().all(|&i| !i) {
        return vec![];
    }
    // Interpolated from the edge's lower (y, then x) node, so the two cells sharing an
    // edge compute the same point bit for bit.
    let crossing = |a: usize, b: usize| {
        let (a, b) = if (corners[a][1], corners[a][0]) <= (corners[b][1], corners[b][0]) {
            (a, b)
        } else {
            (b, a)
        };
        let t = values[a] / (values[a] - values[b]);
        [
            corners[a][0] + (corners[b][0] - corners[a][0]) * t,
            corners[a][1] + (corners[b][1] - corners[a][1]) * t,
        ]
    };
    let saddle = inside == [true, false, true, false] || inside == [false, true, false, true];
    if saddle && centre() < 0.0 {
        let first = if inside[0] { 0 } else { 1 };
        return [first, first + 2]
            .into_iter()
            .map(|k| {
                let (prev, next) = ((k + 3) % 4, (k + 1) % 4);
                vec![corners[k], crossing(k, next), crossing(k, prev)]
            })
            .collect();
    }
    let mut polygon = Vec::with_capacity(8);
    for k in 0..4 {
        let next = (k + 1) % 4;
        if inside[k] {
            polygon.push(corners[k]);
        }
        if inside[k] != inside[next] {
            polygon.push(crossing(k, next));
        }
    }
    vec![polygon]
}

/// Sutherland-Hodgman clip of a convex counter-clockwise polygon by the uv triangle.
fn clip_to_triangle(mut polygon: Vec<[f64; 2]>, uv: &[[f64; 2]; 3], ccw: bool) -> Vec<[f64; 2]> {
    for k in 0..3 {
        let (a, b) = if ccw {
            (uv[k], uv[(k + 1) % 3])
        } else {
            (uv[(k + 1) % 3], uv[k])
        };
        let side = |p: [f64; 2]| cross(sub(b, a), sub(p, a));
        let mut next = Vec::with_capacity(polygon.len() + 2);
        for i in 0..polygon.len() {
            let (p, q) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                next.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                // From the lower endpoint, as cell crossings are.
                let ((p, sp), (q, sq)) = if (p[1], p[0]) <= (q[1], q[0]) { ((p, sp), (q, sq)) } else { ((q, sq), (p, sp)) };
                let t = sp / (sp - sq);
                next.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
            }
        }
        polygon = next;
        if polygon.len() < 3 {
            break;
        }
    }
    polygon
}

/// Fans a convex counter-clockwise uv polygon into triangles with the source winding.
fn push_polygon(
    out: &mut CutoutMesh,
    v: &[Vertex; 3],
    uv: &[[f64; 2]; 3],
    polygon: &[[f64; 2]],
    ccw: bool,
) {
    let base = out.vertices.len() as u32;
    for &p in polygon {
        let w = barycentric(uv, p).unwrap_or([1.0 / 3.0; 3]);
        let mix = |f: &dyn Fn(&Vertex) -> f32| -> f32 {
            (0..3).map(|k| w[k] * f64::from(f(&v[k]))).sum::<f64>() as f32
        };
        let normal = [0, 1, 2].map(|a| mix(&|x| x.normal[a]));
        let length = normal.iter().map(|n| n * n).sum::<f32>().sqrt();
        out.vertices.push(Vertex {
            position: [0, 1, 2].map(|a| mix(&|x| x.position[a])),
            normal: if length > 1e-20 {
                normal.map(|n| n / length)
            } else {
                v[0].normal
            },
            uv: [p[0] as f32, p[1] as f32],
            color: [0, 1, 2, 3].map(|a| mix(&|x| x.color[a])),
        });
    }
    for k in 1..polygon.len() as u32 - 1 {
        if ccw {
            out.indices.extend([base, base + k, base + k + 1]);
        } else {
            out.indices.extend([base, base + k + 1, base + k]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> (Vec<Vertex>, Vec<u32>) {
        let vertex = |x: f32, y: f32| Vertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [x, y],
            color: [1.0; 4],
        };
        (
            vec![vertex(0., 0.), vertex(1., 0.), vertex(1., 1.), vertex(0., 1.)],
            vec![0, 1, 2, 0, 2, 3],
        )
    }
    fn area(mesh: &CutoutMesh) -> f32 {
        mesh.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[t[k] as usize].position);
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * 0.5
            })
            .sum()
    }
    fn disk(size: u32, radius: f32) -> AlphaMask {
        let alpha = (0..size * size)
            .map(|i| {
                let p = [(i % size) as f32 + 0.5, (i / size) as f32 + 0.5].map(|c| c / size as f32 - 0.5);
                f32::from(u8::from(p[0].hypot(p[1]) < radius))
            })
            .collect();
        AlphaMask { width: size, height: size, alpha }
    }

    #[test]
    fn opaque_mask_keeps_the_card_area_and_winding() {
        let (vertices, indices) = quad();
        let mask = AlphaMask { width: 1, height: 1, alpha: vec![1.0] };
        let mesh = cutout(&MeshData { vertices: &vertices, indices: &indices }, &mask, 1.0, 0.5, 16).unwrap();
        // Signed area: every output triangle keeps the source's counter-clockwise winding.
        assert!((area(&mesh) - 1.0).abs() < 1e-5, "{}", area(&mesh));
        let empty = AlphaMask { width: 1, height: 1, alpha: vec![0.0] };
        let mesh = cutout(&MeshData { vertices: &vertices, indices: &indices }, &empty, 1.0, 0.5, 16).unwrap();
        assert!(mesh.indices.is_empty());
    }

    #[test]
    fn cut_cells_weld_into_one_connected_mesh() {
        let (vertices, indices) = quad();
        let mask = disk(256, 0.3);
        let mesh = cutout(&MeshData { vertices: &vertices, indices: &indices }, &mask, 1.0, 0.5, 64).unwrap();
        // Interior edges are shared by two triangles; only the contour is open.
        let mut edges = std::collections::HashMap::new();
        for t in mesh.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]));
                *edges.entry((a, b)).or_insert(0) += 1;
            }
        }
        let open = edges.values().filter(|&&n| n == 1).count();
        assert!(edges.values().all(|&n| n <= 2));
        // The disk's contour is about 2 pi r * 64 cells long: a few hundred open edges.
        assert!(open < 600, "{open} open of {}", edges.len());
    }

    #[test]
    fn disk_mask_cuts_to_its_contour() {
        let (vertices, indices) = quad();
        let mask = disk(256, 0.3);
        let mesh = cutout(&MeshData { vertices: &vertices, indices: &indices }, &mask, 1.0, 0.5, 64).unwrap();
        let expected = std::f32::consts::PI * 0.09;
        assert!((area(&mesh) - expected).abs() < 0.01 * expected, "{} vs {expected}", area(&mesh));
        // Every vertex lies within a cell of the circle or inside it.
        for v in &mesh.vertices {
            let r = (v.position[0] - 0.5).hypot(v.position[1] - 0.5);
            assert!(r < 0.3 + 1.5 / 64.0, "{r}");
        }
    }

    #[test]
    fn mirrored_uv_mapping_keeps_source_winding() {
        let (mut vertices, indices) = quad();
        for v in &mut vertices {
            v.uv[0] = 1.0 - v.uv[0];
        }
        let mask = disk(128, 0.4);
        let mesh = cutout(&MeshData { vertices: &vertices, indices: &indices }, &mask, 1.0, 0.5, 32).unwrap();
        assert!(area(&mesh) > 0.0);
        let expected = std::f32::consts::PI * 0.16;
        assert!((area(&mesh) - expected).abs() < 0.02 * expected, "{}", area(&mesh));
    }

    #[test]
    fn saddle_cells_split_by_their_centre() {
        let polygons = cell_polygons(
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            [1.0, -1.0, 1.0, -1.0],
            || -0.5,
        );
        assert_eq!(polygons.len(), 2);
        let joined = cell_polygons(
            [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            [1.0, -1.0, 1.0, -1.0],
            || 0.5,
        );
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].len(), 6);
    }
}
