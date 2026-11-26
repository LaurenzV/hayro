//! PDF shadings.

#![allow(clippy::needless_range_loop)]

use crate::CacheKey;
use crate::cache::Cache;
use crate::color::{ColorComponents, ColorSpace};
use crate::function::{Function, interpolate};
use crate::util::{PointExt, RectExt};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::Rect;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{
    BACKGROUND, BBOX, BITS_PER_COMPONENT, BITS_PER_COORDINATE, BITS_PER_FLAG, COLORSPACE, COORDS,
    DECODE, DOMAIN, EXTEND, FUNCTION, MATRIX, SHADING_TYPE, VERTICES_PER_ROW,
};
use kurbo::{Affine, BezPath, Point, Shape};
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

/// The function supplied to a shading.
#[derive(Debug, Clone)]
pub enum ShadingFunction {
    /// A single function for all components.
    Single(Function),
    /// Multiple 1-in, 1-out functions, one for each color component.
    Multiple(Vec<Function>),
}

impl ShadingFunction {
    pub(crate) fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Single(a), Self::Single(b)) => a.same(b),
            (Self::Multiple(a), Self::Multiple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same(b))
            }
            _ => false,
        }
    }

    /// The number of output values of the shading function.
    pub fn num_outputs(&self) -> usize {
        match self {
            Self::Single(function) => function.num_outputs(),
            Self::Multiple(functions) => functions.len(),
        }
    }

    /// Evaluate the shading function for a single sample.
    pub fn eval(&self, input: &[f32], output: &mut [f32]) -> Option<()> {
        match self {
            Self::Single(function) => function.eval(input, output),
            Self::Multiple(functions) => {
                if functions.len() != output.len() {
                    return None;
                }

                for (function, output) in functions.iter().zip(output.iter_mut()) {
                    function.eval(input, std::slice::from_mut(output))?;
                }

                Some(())
            }
        }
    }

    pub(crate) fn ops_per_eval(&self) -> usize {
        match self {
            Self::Single(f) => f.ops_per_eval(),
            Self::Multiple(fs) => fs.iter().map(Function::ops_per_eval).sum::<usize>().max(1),
        }
    }

    pub(crate) fn monotone(&self, range: [f64; 2]) -> Option<bool> {
        match self {
            Self::Single(f) => f.monotone(range),
            Self::Multiple(fs) => fs
                .iter()
                .try_fold(true, |ok, f| Some(f.monotone(range)? && ok)),
        }
    }

    pub(crate) fn eval_1d_f64(&self, x: f64, output: &mut [f32]) -> Option<()> {
        match self {
            Self::Single(f) => f.eval_1d_f64(x, output),
            Self::Multiple(fs) if fs.len() == output.len() => {
                for (f, value) in fs.iter().zip(output) {
                    f.eval_1d_f64(x, std::slice::from_mut(value))?;
                }
                Some(())
            }
            _ => None,
        }
    }

    pub(crate) fn linear_pieces(
        &self,
        domain: [f64; 2],
        tolerance: Option<f32>,
        mut emit: impl FnMut([f32; 2], &[f32], &[f32]) -> Option<()>,
    ) -> Option<()> {
        let unit = [0.0, 1.0];
        let mut budget = crate::function::MAX_LINEAR_PIECES;
        match self {
            Self::Single(f) => f.linear_pieces(domain, unit, &mut budget, tolerance, &mut emit),
            Self::Multiple(functions) => {
                if let [f] = functions.as_slice() {
                    return f.linear_pieces(domain, unit, &mut budget, tolerance, &mut emit);
                }
                let mut channels = Vec::new();
                for f in functions {
                    let mut pieces = Vec::new();
                    f.linear_pieces(
                        domain,
                        unit,
                        &mut budget,
                        tolerance,
                        &mut |offsets, a, b| {
                            if offsets[0] >= offsets[1] {
                                return None;
                            }
                            pieces.push((offsets.map(f64::from), a[0], b[0]));
                            Some(())
                        },
                    )?;
                    channels.push(pieces.into_iter().peekable());
                }
                let [mut a, mut b] = [[0.0; crate::function::MAX_COMPONENTS]; 2];
                let mut start = 0.0;
                while start < 1.0 {
                    let mut end = 1.0_f64;
                    for ch in &mut channels {
                        end = end.min(ch.peek()?.0[1]);
                    }
                    for (i, ch) in channels.iter_mut().enumerate() {
                        let &([lo, hi], left, right) = ch.peek()?;
                        let mut value = |t| {
                            if tolerance.is_none() && t != lo && t != hi {
                                let mut v = [0.];
                                functions[i]
                                    .eval_1d_f64(domain[0] + (domain[1] - domain[0]) * t, &mut v)?;
                                budget = budget.checked_sub(1)?;
                                return Some(v[0]);
                            }
                            let u = (t - lo) / (hi - lo);
                            Some(((1.0 - u) * f64::from(left) + u * f64::from(right)) as f32)
                        };
                        a[i] = value(start)?;
                        b[i] = value(end)?;
                        if end == hi {
                            ch.next();
                        }
                    }
                    emit(
                        [start as f32, end as f32],
                        &a[..functions.len()],
                        &b[..functions.len()],
                    )?;
                    start = end;
                }
                Some(())
            }
        }
    }

    pub(crate) fn stitching_bounds(&self) -> Vec<f32> {
        let mut bounds = vec![];

        match self {
            Self::Single(function) => bounds.extend(function.stitching_bounds()),
            Self::Multiple(functions) => {
                for function in functions {
                    bounds.extend(function.stitching_bounds());
                }
            }
        }

        bounds.sort_by(f32::total_cmp);
        bounds.dedup_by(|a, b| (*a - *b).abs() <= f32::EPSILON);
        bounds
    }
}

/// A type of shading.
#[derive(Clone, Debug)]
pub enum ShadingType {
    /// A function-based shading.
    FunctionBased {
        /// The domain of the function.
        domain: [f32; 4],
        /// A transform to apply to the shading.
        matrix: Affine,
        /// The function that should be used to evaluate the shading.
        function: ShadingFunction,
    },
    /// A radial-axial shading.
    RadialAxial {
        /// `x0 y0 x1 y1` for axial shadings, `x0 y0 r0 x1 y1 r1` for radial ones.
        coords: [f32; 6],
        /// The domain of the shading.
        domain: [f32; 2],
        /// The function forming the basis of the shading.
        function: ShadingFunction,
        /// The extends in the left/right direction of the shading.
        extend: [bool; 2],
        /// Whether the shading is axial or radial.
        axial: bool,
    },
    /// A triangle-mesh shading.
    TriangleMesh {
        /// The triangles making up the shading.
        triangles: Vec<Triangle>,
        /// An optional function used for calculating the sampled color values.
        function: Option<ShadingFunction>,
    },
    /// A coons-patch-mesh or tensor-product-patch-mesh shading.
    PatchMesh {
        /// The patches that make up the shading.
        patches: Vec<Patch>,
        /// An optional function used for calculating the sampled color values.
        function: Option<ShadingFunction>,
    },
    /// A dummy shading that should just be drawn transparent.
    Dummy,
}

/// A PDF shading.
#[derive(Clone, Debug)]
pub struct Shading {
    pub(crate) colors: Arc<std::sync::OnceLock<crate::encode::PreparedColors>>,
    precise_coords: Option<Box<[f64; 6]>>,
    cache_key: u128,
    /// The type of shading.
    pub shading_type: Arc<ShadingType>,
    /// The color space of the shading.
    pub color_space: ColorSpace,
    /// A clip path that should be applied to the shading.
    pub clip_path: Option<BezPath>,
    /// The background color of the shading.
    pub background: Option<SmallVec<[f32; 4]>>,
    pub(crate) mesh_bounds: Option<kurbo::Rect>,
}

impl Shading {
    pub(crate) fn coordinates(&self, coords: [f32; 6]) -> [f64; 6] {
        // Callers can replace the public shading type after parsing.
        if let Some(parsed) = &self.precise_coords
            && parsed.map(|v| v as f32) == coords
        {
            return **parsed;
        }
        coords.map(f64::from)
    }

    pub(crate) fn new(dict: &Dict<'_>, stream: Option<&Stream<'_>>, cache: &Cache) -> Option<Self> {
        let cache_key = stream.map_or_else(|| dict.cache_key(), |s| s.cache_key());

        cache.get_or_insert_with(cache_key, || Self::parse(dict, stream, cache, cache_key))
    }

    fn parse(
        dict: &Dict<'_>,
        stream: Option<&Stream<'_>>,
        cache: &Cache,
        cache_key: u128,
    ) -> Option<Self> {
        let shading_num = dict.get::<u8>(SHADING_TYPE)?;

        let color_space = ColorSpace::new(dict.get(COLORSPACE)?, cache)?;

        let mut mesh_bounds = None;
        let mut precise_coords = None;
        let shading_type = match shading_num {
            1 => {
                let domain = dict.get::<[f32; 4]>(DOMAIN).unwrap_or([0.0, 1.0, 0.0, 1.0]);
                let matrix = dict
                    .get::<[f64; 6]>(MATRIX)
                    .map(Affine::new)
                    .unwrap_or_default();
                let function = read_function(dict, &color_space)?;

                ShadingType::FunctionBased {
                    domain,
                    matrix,
                    function,
                }
            }
            2 | 3 => {
                let domain = dict.get::<[f32; 2]>(DOMAIN).unwrap_or([0.0, 1.0]);
                let function = read_function(dict, &color_space)?;
                let extend = dict.get::<[bool; 2]>(EXTEND).unwrap_or([false, false]);
                let (coords, invalid) = if shading_num == 2 {
                    let read = dict.get::<[f64; 4]>(COORDS)?;
                    let invalid = read[0] == read[2] && read[1] == read[3];
                    ([read[0], read[1], read[2], read[3], 0.0, 0.0], invalid)
                } else {
                    let read = dict.get::<[f64; 6]>(COORDS)?;
                    let invalid = read[0] == read[3] && read[1] == read[4] && read[2] == read[5];
                    (read, invalid)
                };

                let axial = shading_num == 2;

                if invalid {
                    ShadingType::Dummy
                } else {
                    let rounded = coords.map(|x| x as f32);
                    if rounded.map(f64::from) != coords {
                        precise_coords = Some(coords);
                    }
                    ShadingType::RadialAxial {
                        domain,
                        function,
                        extend,
                        coords: rounded,
                        axial,
                    }
                }
            }
            4 => {
                let stream = stream?;
                let stream_data = stream.decoded_data().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
                let function = read_function(dict, &color_space);
                let decode = dict
                    .get::<Array<'_>>(DECODE)?
                    .iter::<f32>()
                    .collect::<Vec<_>>();

                let (triangles, bounds) = read_free_form_triangles(
                    stream_data.as_ref(),
                    bpf,
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    &decode,
                )?;
                mesh_bounds = bounds;

                ShadingType::TriangleMesh {
                    triangles,
                    function,
                }
            }
            5 => {
                let stream = stream?;
                let stream_data = stream.decoded_data().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let function = read_function(dict, &color_space);
                let decode = dict
                    .get::<Array<'_>>(DECODE)?
                    .iter::<f32>()
                    .collect::<Vec<_>>();
                let vertices_per_row = dict.get::<u32>(VERTICES_PER_ROW)?;

                let (triangles, bounds) = read_lattice_triangles(
                    stream_data.as_ref(),
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    vertices_per_row,
                    &decode,
                )?;
                mesh_bounds = bounds;

                ShadingType::TriangleMesh {
                    triangles,
                    function,
                }
            }
            6 => {
                let stream = stream?;
                let stream_data = stream.decoded_data().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
                let function = read_function(dict, &color_space);
                let decode = dict
                    .get::<Array<'_>>(DECODE)?
                    .iter::<f32>()
                    .collect::<Vec<_>>();

                let (patches, bounds) = read_patch_mesh(
                    stream_data.as_ref(),
                    bpf,
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    &decode,
                    PatchKind::Coons,
                )?;
                mesh_bounds = bounds;

                ShadingType::PatchMesh { patches, function }
            }
            7 => {
                let stream = stream?;
                let stream_data = stream.decoded_data().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
                let function = read_function(dict, &color_space);
                let decode = dict
                    .get::<Array<'_>>(DECODE)?
                    .iter::<f32>()
                    .collect::<Vec<_>>();

                let (patches, bounds) = read_patch_mesh(
                    stream_data.as_ref(),
                    bpf,
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    &decode,
                    PatchKind::TensorProduct,
                )?;
                mesh_bounds = bounds;

                ShadingType::PatchMesh { patches, function }
            }
            _ => return None,
        };

        let bbox = dict.get::<Rect>(BBOX).map(|r| r.to_kurbo());
        let background = dict
            .get::<Array<'_>>(BACKGROUND)
            .map(|a| a.iter::<f32>().collect::<SmallVec<_>>());

        let shading_type = Arc::new(shading_type);
        Some(Self {
            colors: Arc::default(),
            precise_coords: precise_coords.map(Box::new),
            cache_key,
            shading_type,
            color_space,
            clip_path: bbox.map(|r| r.to_path(0.1)),
            background,
            mesh_bounds,
        })
    }
}

impl CacheKey for Shading {
    fn cache_key(&self) -> u128 {
        self.cache_key
    }
}

/// A triangle of a triangle-mesh shading.
#[derive(Clone, Debug)]
pub struct Triangle {
    /// The vertices of the triangle.
    pub vertices: [TriangleVertex; 3],
}

/// A triangle vertex.
#[derive(Clone, Debug)]
pub struct TriangleVertex {
    flag: u32,
    /// The position of the vertex.
    pub point: Point,
    /// The color components of the vertex.
    pub colors: ColorComponents,
}

/// A patch of a patch-mesh shading in tensor-product form (Coons patches are converted).
#[derive(Clone, Debug)]
pub struct Patch {
    /// The control points, indexed by column (`u`) and then by row (`v`).
    pub control_points: [[Point; 4]; 4],
    /// The colors at the corners, indexed by `u` and then by `v`.
    pub colors: [[ColorComponents; 2]; 2],
}

fn read_free_form_triangles(
    data: &[u8],
    bpf: u8,
    bp_cord: u8,
    bp_comp: u8,
    has_function: bool,
    decode: &[f32],
) -> Option<(Vec<Triangle>, Option<kurbo::Rect>)> {
    let mut triangles = vec![];
    let mut bounds = None;

    let ([x_min, x_max, y_min, y_max], decode) = split_decode(decode)?;
    let mut reader = BitReader::new(data);
    let helpers = InterpolationHelpers::new(bp_cord, bp_comp, x_min, x_max, y_min, y_max);

    let mut read_single = |reader: &mut BitReader<'_>| -> Option<TriangleVertex> {
        let vertex = helpers.read_triangle_vertex(reader, bpf, has_function, decode)?;
        extend_bounds(
            &mut bounds,
            kurbo::Rect::from_points(vertex.point, vertex.point),
        );
        Some(vertex)
    };

    let mut a = None;
    let mut b = None;
    let mut c = None;

    loop {
        let Some(first) = read_single(&mut reader) else {
            break;
        };

        if first.flag == 0 {
            let second = read_single(&mut reader)?;
            let third = read_single(&mut reader)?;

            a = Some(first.clone());
            b = Some(second.clone());
            c = Some(third.clone());
        } else if first.flag == 1 {
            a = Some(b.clone()?);
            b = Some(c.clone()?);
            c = Some(first);
        } else if first.flag == 2 {
            b = Some(c.clone()?);
            c = Some(first);
        }

        let (p0, p1, p2) = (a.clone()?, b.clone()?, c.clone()?);

        if p0.point.nearly_same(p1.point) || p1.point.nearly_same(p2.point) {
            continue;
        }

        triangles.push(Triangle {
            vertices: [p0, p1, p2],
        });
    }

    Some((triangles, bounds))
}

struct InterpolationHelpers {
    bp_coord: u8,
    bp_comp: u8,
    coord_max: f32,
    comp_max: f32,
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
}

impl InterpolationHelpers {
    fn new(bp_coord: u8, bp_comp: u8, x_min: f32, x_max: f32, y_min: f32, y_max: f32) -> Self {
        let coord_max = 2.0_f32.powi(bp_coord as i32) - 1.0;
        let comp_max = 2.0_f32.powi(bp_comp as i32) - 1.0;
        Self {
            bp_coord,
            bp_comp,
            coord_max,
            comp_max,
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }

    fn interpolate_coord(&self, n: u32, d_min: f32, d_max: f32) -> f32 {
        interpolate(n as f32, 0.0, self.coord_max, d_min, d_max)
    }

    fn interpolate_comp(&self, n: u32, d_min: f32, d_max: f32) -> f32 {
        interpolate(n as f32, 0.0, self.comp_max, d_min, d_max)
    }

    fn read_point(&self, reader: &mut BitReader<'_>) -> Option<Point> {
        let x = self.interpolate_coord(reader.read(self.bp_coord)?, self.x_min, self.x_max);
        let y = self.interpolate_coord(reader.read(self.bp_coord)?, self.y_min, self.y_max);
        Some(Point::new(x as f64, y as f64))
    }

    fn read_colors(
        &self,
        reader: &mut BitReader<'_>,
        has_function: bool,
        decode: &[f32],
    ) -> Option<ColorComponents> {
        let mut colors = smallvec![];
        if has_function {
            colors.push(self.interpolate_comp(
                reader.read(self.bp_comp)?,
                *decode.first()?,
                *decode.get(1)?,
            ));
        } else {
            let num_components = decode.len() / 2;
            for (_, decode) in (0..num_components).zip(decode.as_chunks::<2>().0) {
                colors.push(self.interpolate_comp(
                    reader.read(self.bp_comp)?,
                    decode[0],
                    decode[1],
                ));
            }
        }
        Some(colors)
    }

    fn read_triangle_vertex(
        &self,
        reader: &mut BitReader<'_>,
        bpf: u8,
        has_function: bool,
        decode: &[f32],
    ) -> Option<TriangleVertex> {
        let flag = reader.read(bpf)?;
        let point = self.read_point(reader)?;
        let colors = self.read_colors(reader, has_function, decode)?;
        reader.align();

        Some(TriangleVertex {
            flag,
            point,
            colors,
        })
    }
}

fn split_decode(decode: &[f32]) -> Option<([f32; 4], &[f32])> {
    decode.split_first_chunk::<4>().map(|(a, b)| (*a, b))
}

fn read_lattice_triangles(
    data: &[u8],
    bp_cord: u8,
    bp_comp: u8,
    has_function: bool,
    vertices_per_row: u32,
    decode: &[f32],
) -> Option<(Vec<Triangle>, Option<kurbo::Rect>)> {
    if vertices_per_row < 2 {
        return None;
    }

    let mut lattices = vec![];
    let mut bounds = None;

    let ([x_min, x_max, y_min, y_max], decode) = split_decode(decode)?;
    let mut reader = BitReader::new(data);
    let helpers = InterpolationHelpers::new(bp_cord, bp_comp, x_min, x_max, y_min, y_max);

    let mut read_single = |reader: &mut BitReader<'_>| -> Option<TriangleVertex> {
        let point = helpers.read_point(reader)?;
        let colors = helpers.read_colors(reader, has_function, decode)?;
        reader.align();
        extend_bounds(&mut bounds, kurbo::Rect::from_points(point, point));

        Some(TriangleVertex {
            flag: 0,
            point,
            colors,
        })
    };

    'outer: loop {
        let mut single_row = vec![];

        for _ in 0..vertices_per_row {
            let Some(next) = read_single(&mut reader) else {
                break 'outer;
            };

            single_row.push(next);
        }

        lattices.push(single_row);
    }

    let mut triangles = vec![];

    for i in 0..lattices.len().saturating_sub(1) {
        for j in 0..(vertices_per_row as usize).saturating_sub(1) {
            triangles.push(Triangle {
                vertices: [
                    lattices[i][j].clone(),
                    lattices[i + 1][j].clone(),
                    lattices[i][j + 1].clone(),
                ],
            });

            triangles.push(Triangle {
                vertices: [
                    lattices[i + 1][j + 1].clone(),
                    lattices[i + 1][j].clone(),
                    lattices[i][j + 1].clone(),
                ],
            });
        }
    }

    Some((triangles, bounds))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PatchKind {
    Coons,
    TensorProduct,
}

#[allow(clippy::too_many_arguments)]
fn read_patch_mesh(
    data: &[u8],
    bpf: u8,
    bp_coord: u8,
    bp_comp: u8,
    has_function: bool,
    decode: &[f32],
    kind: PatchKind,
) -> Option<(Vec<Patch>, Option<kurbo::Rect>)> {
    let control_points_count = match kind {
        PatchKind::Coons => 12,
        PatchKind::TensorProduct => 16,
    };
    let mut bounds = None;
    let ([x_min, x_max, y_min, y_max], decode) = split_decode(decode)?;
    let mut reader = BitReader::new(data);
    let helpers = InterpolationHelpers::new(bp_coord, bp_comp, x_min, x_max, y_min, y_max);

    let read_colors = |reader: &mut BitReader<'_>| -> Option<ColorComponents> {
        helpers.read_colors(reader, has_function, decode)
    };

    let mut prev_patch_points: Option<[Point; 16]> = None;
    let mut prev_patch_colors: Option<[ColorComponents; 4]> = None;
    let mut patches = vec![];

    while let Some(flag) = reader.read(bpf) {
        let mut control_points = [Point::ZERO; 16];
        let mut colors = [smallvec![], smallvec![], smallvec![], smallvec![]];

        match flag {
            0 => {
                for i in 0..control_points_count {
                    control_points[i] = helpers.read_point(&mut reader)?;
                }

                for i in 0..4 {
                    colors[i] = read_colors(&mut reader)?;
                }

                prev_patch_points = Some(control_points);
                prev_patch_colors = Some(colors.clone());
            }
            1..=3 => {
                let prev_points = prev_patch_points?;
                let prev_colors = prev_patch_colors.as_ref()?;

                copy_patch_control_points(flag, &prev_points, &mut control_points);

                match flag {
                    1 => {
                        colors[0] = prev_colors[1].clone();
                        colors[1] = prev_colors[2].clone();
                    }
                    2 => {
                        colors[0] = prev_colors[2].clone();
                        colors[1] = prev_colors[3].clone();
                    }
                    3 => {
                        colors[0] = prev_colors[3].clone();
                        colors[1] = prev_colors[0].clone();
                    }
                    _ => unreachable!(),
                }

                for i in 4..control_points_count {
                    control_points[i] = helpers.read_point(&mut reader)?;
                }

                colors[2] = read_colors(&mut reader)?;
                colors[3] = read_colors(&mut reader)?;

                prev_patch_points = Some(control_points);
                prev_patch_colors = Some(colors.clone());
            }
            _ => break,
        }

        let patch = patch_from_stream(&control_points, colors, kind);
        extend_bounds(
            &mut bounds,
            points_bbox(patch.control_points.as_flattened()),
        );
        patches.push(patch);
    }

    Some((patches, bounds))
}

const STREAM_ORDER: [[usize; 4]; 4] =
    [[0, 1, 2, 3], [11, 12, 13, 4], [10, 15, 14, 5], [9, 8, 7, 6]];

fn patch_from_stream(points: &[Point; 16], colors: [ColorComponents; 4], kind: PatchKind) -> Patch {
    let mut p = STREAM_ORDER.map(|column| column.map(|i| points[i]));

    if kind == PatchKind::Coons {
        let [p00, p01, p02, p03] = p[0].map(Point::to_vec2);
        let [p10, _, _, p13] = p[1].map(Point::to_vec2);
        let [p20, _, _, p23] = p[2].map(Point::to_vec2);
        let [p30, p31, p32, p33] = p[3].map(Point::to_vec2);

        p[1][1] = ((-4.0 * p00 + 6.0 * (p01 + p10) - 2.0 * (p03 + p30) + 3.0 * (p31 + p13) - p33)
            / 9.0)
            .to_point();
        p[1][2] = ((-4.0 * p03 + 6.0 * (p02 + p13) - 2.0 * (p00 + p33) + 3.0 * (p32 + p10) - p30)
            / 9.0)
            .to_point();
        p[2][1] = ((-4.0 * p30 + 6.0 * (p31 + p20) - 2.0 * (p33 + p00) + 3.0 * (p01 + p23) - p03)
            / 9.0)
            .to_point();
        p[2][2] = ((-4.0 * p33 + 6.0 * (p32 + p23) - 2.0 * (p30 + p03) + 3.0 * (p02 + p20) - p00)
            / 9.0)
            .to_point();
    }

    let [c0, c1, c2, c3] = colors;

    Patch {
        control_points: p,
        colors: [[c0, c1], [c3, c2]],
    }
}

fn copy_patch_control_points(
    flag: u32,
    prev_control_points: &[Point; 16],
    control_points: &mut [Point; 16],
) {
    match flag {
        1 => {
            control_points[0] = prev_control_points[3];
            control_points[1] = prev_control_points[4];
            control_points[2] = prev_control_points[5];
            control_points[3] = prev_control_points[6];
        }
        2 => {
            control_points[0] = prev_control_points[6];
            control_points[1] = prev_control_points[7];
            control_points[2] = prev_control_points[8];
            control_points[3] = prev_control_points[9];
        }
        3 => {
            control_points[0] = prev_control_points[9];
            control_points[1] = prev_control_points[10];
            control_points[2] = prev_control_points[11];
            control_points[3] = prev_control_points[0];
        }
        _ => {}
    }
}

fn read_function(dict: &Dict<'_>, color_space: &ColorSpace) -> Option<ShadingFunction> {
    if let Some(arr) = dict.get::<Array<'_>>(FUNCTION) {
        let arr = arr
            .iter::<Object<'_>>()
            .map(|o| Function::new(&o))
            .collect::<Option<Vec<_>>>()?;

        if arr.len() != color_space.num_components() as usize {
            return None;
        }

        if arr.iter().any(|f| f.num_outputs() != 1) {
            return None;
        }

        Some(ShadingFunction::Multiple(arr))
    } else if let Some(obj) = dict.get::<Object<'_>>(FUNCTION) {
        Some(ShadingFunction::Single(Function::new(&obj)?))
    } else {
        None
    }
}

fn extend_bounds(bounds: &mut Option<kurbo::Rect>, rect: kurbo::Rect) {
    *bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
}

fn points_bbox(points: &[Point]) -> kurbo::Rect {
    points
        .iter()
        .fold(kurbo::Rect::from_points(points[0], points[0]), |bbox, p| {
            bbox.union_pt(*p)
        })
}

#[cfg(test)]
mod tests {
    use super::{Function, ShadingFunction, read_lattice_triangles};
    use hayro_syntax::object::{Dict, FromBytes, Object};

    const DECODE: [f32; 6] = [0.0, 100.0, 0.0, 100.0, 0.0, 1.0];

    #[test]
    fn channels_with_distinct_stitches_split_the_other_channels_ramp() {
        let line = "<< /FunctionType 2 /Domain [0 1] /N 1 >>";
        let parse = |s: &str| {
            Function::new(&Object::Dict(Dict::from_bytes(s.as_bytes()).unwrap())).unwrap()
        };
        let step = |at| {
            parse(&format!(
                "<< /FunctionType 3 /Domain [0 1] /Functions [{line} {line}] /Bounds [{at}] /Encode [0 0 1 1] >>"
            ))
        };
        let function = ShadingFunction::Multiple(vec![step(0.25), parse(line), step(0.75)]);
        let mut pieces = Vec::new();
        function
            .linear_pieces([0.0, 1.0], Some(0.01), |offsets, a, b| {
                pieces.push((
                    offsets,
                    <[f32; 3]>::try_from(a).unwrap(),
                    <[f32; 3]>::try_from(b).unwrap(),
                ));
                Some(())
            })
            .unwrap();
        assert_eq!(
            pieces,
            [
                ([0.0, 0.25], [0.0; 3], [0.0, 0.25, 0.0]),
                ([0.25, 0.75], [1.0, 0.25, 0.0], [1.0, 0.75, 0.0]),
                ([0.75, 1.0], [1.0, 0.75, 1.0], [1.0; 3]),
            ]
        );
    }

    #[test]
    fn lattice_rejects_rows_of_fewer_than_two_vertices() {
        assert!(read_lattice_triangles(&[], 8, 8, false, 0, &DECODE).is_none());
        assert!(read_lattice_triangles(&[0; 3], 8, 8, false, 1, &DECODE).is_none());
    }
}
