/// PDF shadings.
use crate::color::{ColorComponents, ColorSpace};
use crate::util::{FloatExt, PointExt};
use hayro_syntax::bit_reader::{BitReader, BitSize};
use hayro_syntax::function::{Function, Values, interpolate};
use hayro_syntax::object::Array;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Object;
use hayro_syntax::object::Rect;
use hayro_syntax::object::Stream;
use hayro_syntax::object::dict::keys::{
    BACKGROUND, BBOX, BITS_PER_COMPONENT, BITS_PER_COORDINATE, BITS_PER_FLAG, COLORSPACE, COORDS,
    DECODE, DOMAIN, EXTEND, FUNCTION, MATRIX, SHADING_TYPE, VERTICES_PER_ROW,
};
use kurbo::{Affine, CubicBez, ParamCurve, Point, Shape};
use log::warn;
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

/// The function supplied to a shading.
#[derive(Debug, Clone)]
pub enum ShadingFunction {
    /// A single function, which should be used to evaluate all components of the shading.
    Single(Function),
    /// Multiple functions, one for each color component.
    Multiple(SmallVec<[Function; 4]>),
}

impl ShadingFunction {
    /// Evaluate the shading function.
    pub fn eval(&self, input: &Values) -> Option<Values> {
        match self {
            ShadingFunction::Single(s) => s.eval(input.clone()),
            ShadingFunction::Multiple(m) => {
                // 1-in, 1-out function for each color component.

                let mut out = smallvec![];

                for func in m {
                    out.push(*func.eval(input.clone())?.first()?);
                }

                Some(out)
            }
        }
    }
}

/// A type of shading
#[derive(Debug)]
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
        /// The coordinates of the shading.
        ///
        /// For axial shadings, only the first 4 entries are relevant, representing the x/y coordinates
        /// of the first point and the coordinates for the second point.
        ///
        /// For radial shadings, the coordinates contain the x/y coordinates as well as the radius
        /// for both circles.
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
    /// A coons-patch-mesh shading.
    CoonsPatchMesh {
        /// The patches that make up the shading.
        patches: Vec<CoonsPatch>,
        /// An optional function used for calculating the sampled color values.
        function: Option<ShadingFunction>,
    },
    /// A tensor-product-patch-mesh shading.
    TensorProductPatchMesh {
        /// The patches that make up the shading.
        patches: Vec<TensorProductPatch>,
        /// An optional function used for calculating the sampled color values.
        function: Option<ShadingFunction>,
    },
    Dummy,
}

/// A PDF shading.
#[derive(Clone, Debug)]
pub struct Shading {
    /// The type of shading.
    pub shading_type: Arc<ShadingType>,
    /// The color space of the shading.
    pub color_space: ColorSpace,
    /// The bounding box of the shading.
    pub bbox: Option<Rect>,
    /// The background color of the shading.
    pub background: Option<SmallVec<[f32; 4]>>,
}

impl Shading {
    pub fn new(dict: &Dict, stream: Option<&Stream>) -> Option<Self> {
        let shading_num = dict.get::<u8>(SHADING_TYPE)?;

        let color_space = ColorSpace::new(dict.get(COLORSPACE)?)?;

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
                    let read = dict.get::<[f32; 4]>(COORDS)?;
                    let invalid = (read[0] - read[2]).is_nearly_zero()
                        && (read[1] - read[3]).is_nearly_zero();
                    ([read[0], read[1], read[2], read[3], 0.0, 0.0], invalid)
                } else {
                    let read = dict.get::<[f32; 6]>(COORDS)?;
                    let invalid = (read[0] - read[3]).is_nearly_zero()
                        && (read[1] - read[4]).is_nearly_zero()
                        && (read[2] - read[5]).is_nearly_zero();
                    (read, invalid)
                };

                let axial = shading_num == 2;

                if invalid {
                    ShadingType::Dummy
                } else {
                    ShadingType::RadialAxial {
                        domain,
                        function,
                        extend,
                        coords,
                        axial,
                    }
                }
            }
            4 => {
                let stream = stream?;
                let stream_data = stream.decoded().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
                let function = read_function(dict, &color_space);
                let decode = dict.get::<Array>(DECODE)?.iter::<f32>().collect::<Vec<_>>();

                let triangles = read_free_form_triangles(
                    stream_data.as_ref(),
                    bpf,
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    &decode,
                )?;

                ShadingType::TriangleMesh {
                    triangles,
                    function,
                }
            }
            5 => {
                let stream = stream?;
                let stream_data = stream.decoded().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let function = read_function(dict, &color_space);
                let decode = dict.get::<Array>(DECODE)?.iter::<f32>().collect::<Vec<_>>();
                let vertices_per_row = dict.get::<u32>(VERTICES_PER_ROW)?;

                let triangles = read_lattice_triangles(
                    stream_data.as_ref(),
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    vertices_per_row,
                    &decode,
                )?;

                ShadingType::TriangleMesh {
                    triangles,
                    function,
                }
            }
            6 => {
                let stream = stream?;
                let stream_data = stream.decoded().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
                let function = read_function(dict, &color_space);
                let decode = dict.get::<Array>(DECODE)?.iter::<f32>().collect::<Vec<_>>();

                let patches = read_coons_patch_mesh(
                    stream_data.as_ref(),
                    bpf,
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    &decode,
                )?;

                ShadingType::CoonsPatchMesh { patches, function }
            }
            7 => {
                let stream = stream?;
                let stream_data = stream.decoded().ok()?;
                let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
                let function = read_function(dict, &color_space);
                let decode = dict.get::<Array>(DECODE)?.iter::<f32>().collect::<Vec<_>>();

                let patches = read_tensor_product_patch_mesh(
                    stream_data.as_ref(),
                    bpf,
                    bp_coord,
                    bp_comp,
                    function.is_some(),
                    &decode,
                )?;

                ShadingType::TensorProductPatchMesh { patches, function }
            }
            _ => return None,
        };

        let bbox = dict.get::<Rect>(BBOX);
        let background = dict
            .get::<Array>(BACKGROUND)
            .map(|a| a.iter::<f32>().collect::<SmallVec<_>>());

        Some(Self {
            shading_type: Arc::new(shading_type),
            color_space,
            bbox,
            background,
        })
    }
}

/// A triangle made up of three vertices.
#[derive(Clone, Debug)]
pub struct Triangle {
    /// The first vertex.
    pub p0: TriangleVertex,
    /// The second vertex.
    pub p1: TriangleVertex,
    /// The third vertex.
    pub p2: TriangleVertex,
    kurbo_tri: kurbo::Triangle,
    d00: f64,
    d01: f64,
    d11: f64,
}

impl Triangle {
    pub fn new(p0: TriangleVertex, p1: TriangleVertex, p2: TriangleVertex) -> Self {
        let v0 = p1.point - p0.point;
        let v1 = p2.point - p0.point;

        let d00 = v0.dot(v0);
        let d01 = v0.dot(v1);
        let d11 = v1.dot(v1);

        let kurbo_tri = kurbo::Triangle::new(p0.point, p1.point, p2.point);

        Self {
            p0,
            p1,
            kurbo_tri,
            p2,
            d00,
            d01,
            d11,
        }
    }
    /// Get the interpolated colors of the point from the triangle.
    ///
    /// Returns `None` if the point is not inside of the triangle.
    pub fn interpolate(&self, pos: Point) -> ColorComponents {
        let (u, v, w) = self.barycentric_coords(pos);

        let mut result = smallvec![];

        for i in 0..self.p0.colors.len() {
            let c0 = self.p0.colors[i];
            let c1 = self.p1.colors[i];
            let c2 = self.p2.colors[i];
            result.push(u * c0 + v * c1 + w * c2);
        }

        result
    }

    pub fn contains_point(&self, pos: Point) -> bool {
        self.kurbo_tri.winding(pos) != 0
    }

    pub fn bounding_box(&self) -> kurbo::Rect {
        self.kurbo_tri.bounding_box()
    }

    /// Return the barycentric coordinates of the point in the triangle.
    ///
    /// Returns `None` if the point is not inside of the triangle.
    fn barycentric_coords(&self, p: Point) -> (f32, f32, f32) {
        let (a, b, c) = (self.p0.point, self.p1.point, self.p2.point);
        let v0 = b - a;
        let v1 = c - a;
        let v2 = p - a;

        let d00 = self.d00;
        let d01 = self.d01;
        let d11 = self.d11;
        let d20 = v2.dot(v0);
        let d21 = v2.dot(v1);

        let denom = d00 * d11 - d01 * d01;
        let v = (d11 * d20 - d01 * d21) / denom;
        let w = (d00 * d21 - d01 * d20) / denom;
        let u = (1.0 - v - w) as f32;

        (u, v as f32, w as f32)
    }
}

/// A triangle vertex.
#[derive(Clone, Debug)]
pub struct TriangleVertex {
    flag: u32,
    /// The position of the vertex.
    pub point: Point,
    /// The color component of the vertex.
    pub colors: ColorComponents,
}

/// A coons patch.
#[derive(Clone, Debug)]
pub struct CoonsPatch {
    /// The control points of the coons patch.
    pub control_points: [Point; 12],
    /// The colors at each corner of the coons patch.
    pub colors: [ColorComponents; 4],
}

/// A tensor-product patch.
#[derive(Clone, Debug)]
pub struct TensorProductPatch {
    /// The control points of the tensor-product patch (4x4 grid = 16 points).
    pub control_points: [Point; 16],
    /// The colors at each corner of the tensor-product patch.
    pub colors: [ColorComponents; 4],
}

impl CoonsPatch {
    /// Map a coordinate from the unit square of the patch to it's actual coordinate.
    pub fn map_coordinate(&self, p: Point) -> Point {
        let (u, v) = (p.x, p.y);

        let cp = &self.control_points;

        let c1 = CubicBez::new(cp[0], cp[11], cp[10], cp[9]);
        let c2 = CubicBez::new(cp[3], cp[4], cp[5], cp[6]);
        let d1 = CubicBez::new(cp[0], cp[1], cp[2], cp[3]);
        let d2 = CubicBez::new(cp[9], cp[8], cp[7], cp[6]);

        let sc = (1.0 - v) * c1.eval(u).to_vec2() + v * c2.eval(u).to_vec2();
        let sd = (1.0 - u) * d1.eval(v).to_vec2() + u * d2.eval(v).to_vec2();
        let sb = (1.0 - v) * ((1.0 - u) * c1.eval(0.0).to_vec2() + u * c1.eval(1.0).to_vec2())
            + v * ((1.0 - u) * c2.eval(0.0).to_vec2() + u * c2.eval(1.0).to_vec2());

        (sc + sd - sb).to_point()
    }

    pub fn to_triangles(&self) -> Vec<Triangle> {
        const GRID_SIZE: usize = 20;
        let mut grid = vec![vec![Point::ZERO; GRID_SIZE]; GRID_SIZE];

        // Create 20x20 grid by mapping unit square coordinates
        for i in 0..GRID_SIZE {
            for j in 0..GRID_SIZE {
                let u = i as f64 / (GRID_SIZE - 1) as f64; // 0.0 to 1.0 (left to right)
                let v = j as f64 / (GRID_SIZE - 1) as f64; // 0.0 to 1.0 (top to bottom)

                // Map unit square coordinate to patch coordinate
                let unit_point = Point::new(u, v);
                grid[i][j] = self.map_coordinate(unit_point);
            }
        }

        // Create triangles from adjacent grid points
        let mut triangles = vec![];

        for i in 0..(GRID_SIZE - 1) {
            for j in 0..(GRID_SIZE - 1) {
                let p00 = grid[i][j];
                let p10 = grid[i + 1][j];
                let p01 = grid[i][j + 1];
                let p11 = grid[i + 1][j + 1];

                // Calculate unit square coordinates for color interpolation
                let u0 = i as f64 / (GRID_SIZE - 1) as f64;
                let u1 = (i + 1) as f64 / (GRID_SIZE - 1) as f64;
                let v0 = j as f64 / (GRID_SIZE - 1) as f64;
                let v1 = (j + 1) as f64 / (GRID_SIZE - 1) as f64;

                // Create triangle vertices with interpolated colors
                let v00 = TriangleVertex {
                    flag: 0,
                    point: p00,
                    colors: self.interpolate(Point::new(u0, v0)),
                };
                let v10 = TriangleVertex {
                    flag: 0,
                    point: p10,
                    colors: self.interpolate(Point::new(u1, v0)),
                };
                let v01 = TriangleVertex {
                    flag: 0,
                    point: p01,
                    colors: self.interpolate(Point::new(u0, v1)),
                };
                let v11 = TriangleVertex {
                    flag: 0,
                    point: p11,
                    colors: self.interpolate(Point::new(u1, v1)),
                };

                triangles.push(Triangle::new(v00.clone(), v10.clone(), v01.clone()));
                triangles.push(Triangle::new(v10.clone(), v11.clone(), v01.clone()));
            }
        }

        triangles
    }

    /// Get the interpolated colors of the point from the patch.
    pub fn interpolate(&self, pos: Point) -> ColorComponents {
        let (u, v) = (pos.x, pos.y);
        let (c0, c1, c2, c3) = {
            (
                &self.colors[0],
                &self.colors[1],
                &self.colors[2],
                &self.colors[3],
            )
        };

        let mut result = SmallVec::new();
        for i in 0..c0.len() {
            let val = (1.0 - u) * (1.0 - v) * c0[i] as f64
                + u * (1.0 - v) * c3[i] as f64
                + u * v * c2[i] as f64
                + (1.0 - u) * v * c1[i] as f64;
            result.push(val as f32);
        }

        result
    }
}

impl TensorProductPatch {
    /// Evaluate Bernstein polynomial B_i(t) for tensor-product patches.
    fn bernstein(i: usize, t: f64) -> f64 {
        match i {
            0 => (1.0 - t).powi(3),
            1 => 3.0 * t * (1.0 - t).powi(2),
            2 => 3.0 * t.powi(2) * (1.0 - t),
            3 => t.powi(3),
            _ => 0.0,
        }
    }

    /// Map a coordinate from the unit square of the patch to its actual coordinate using tensor-product formula.
    pub fn map_coordinate(&self, p: Point) -> Point {
        let (u, v) = (p.x, p.y);

        let mut x = 0.0;
        let mut y = 0.0;

        fn idx(i: usize, j: usize) -> usize {
            match (i, j) {
                (0, 0) => 0,
                (0, 1) => 1,
                (0, 2) => 2,
                (0, 3) => 3,
                (1, 0) => 11,
                (1, 1) => 12,
                (1, 2) => 13,
                (1, 3) => 4,
                (2, 0) => 10,
                (2, 1) => 15,
                (2, 2) => 14,
                (2, 3) => 5,
                (3, 0) => 9,
                (3, 1) => 8,
                (3, 2) => 7,
                (3, 3) => 6,
                _ => panic!("Invalid index"),
            }
        }

        for i in 0..4 {
            for j in 0..4 {
                let control_point_idx = idx(i, j);
                let basis = Self::bernstein(i, u) * Self::bernstein(j, v);

                x += self.control_points[control_point_idx].x * basis;
                y += self.control_points[control_point_idx].y * basis;
            }
        }

        Point::new(x, y)
    }

    pub fn to_triangles(&self) -> Vec<Triangle> {
        const GRID_SIZE: usize = 20;
        let mut grid = vec![vec![Point::ZERO; GRID_SIZE]; GRID_SIZE];

        // Create grid by mapping unit square coordinates
        for i in 0..GRID_SIZE {
            for j in 0..GRID_SIZE {
                let u = i as f64 / (GRID_SIZE - 1) as f64; // 0.0 to 1.0 (left to right)
                let v = j as f64 / (GRID_SIZE - 1) as f64; // 0.0 to 1.0 (top to bottom)

                // Map unit square coordinate to patch coordinate
                let unit_point = Point::new(u, v);
                grid[i][j] = self.map_coordinate(unit_point);
            }
        }

        // Create triangles from adjacent grid points
        let mut triangles = vec![];

        for i in 0..(GRID_SIZE - 1) {
            for j in 0..(GRID_SIZE - 1) {
                let p00 = grid[i][j];
                let p10 = grid[i + 1][j];
                let p01 = grid[i][j + 1];
                let p11 = grid[i + 1][j + 1];

                // Calculate unit square coordinates for color interpolation
                let u0 = i as f64 / (GRID_SIZE - 1) as f64;
                let u1 = (i + 1) as f64 / (GRID_SIZE - 1) as f64;
                let v0 = j as f64 / (GRID_SIZE - 1) as f64;
                let v1 = (j + 1) as f64 / (GRID_SIZE - 1) as f64;

                // Create triangle vertices with interpolated colors
                let v00 = TriangleVertex {
                    flag: 0,
                    point: p00,
                    colors: self.interpolate(Point::new(u0, v0)),
                };
                let v10 = TriangleVertex {
                    flag: 0,
                    point: p10,
                    colors: self.interpolate(Point::new(u1, v0)),
                };
                let v01 = TriangleVertex {
                    flag: 0,
                    point: p01,
                    colors: self.interpolate(Point::new(u0, v1)),
                };
                let v11 = TriangleVertex {
                    flag: 0,
                    point: p11,
                    colors: self.interpolate(Point::new(u1, v1)),
                };

                triangles.push(Triangle::new(v00.clone(), v10.clone(), v01.clone()));
                triangles.push(Triangle::new(v10.clone(), v11.clone(), v01.clone()));
            }
        }

        triangles
    }

    /// Get the interpolated colors of the point from the patch.
    pub fn interpolate(&self, pos: Point) -> ColorComponents {
        let (u, v) = (pos.x, pos.y);
        let (c0, c1, c2, c3) = {
            (
                &self.colors[0],
                &self.colors[1],
                &self.colors[2],
                &self.colors[3],
            )
        };

        let mut result = SmallVec::new();
        for i in 0..c0.len() {
            let val = (1.0 - u) * (1.0 - v) * c0[i] as f64
                + u * (1.0 - v) * c3[i] as f64
                + u * v * c2[i] as f64
                + (1.0 - u) * v * c1[i] as f64;
            result.push(val as f32);
        }

        result
    }
}

fn read_free_form_triangles(
    data: &[u8],
    bpf: u8,
    bp_cord: u8,
    bp_comp: u8,
    has_function: bool,
    decode: &[f32],
) -> Option<Vec<Triangle>> {
    let bpf = BitSize::from_u8(bpf)?;
    let bp_cord = BitSize::from_u8(bp_cord)?;
    let bp_comp = BitSize::from_u8(bp_comp)?;

    let mut triangles = vec![];

    let ([x_min, x_max, y_min, y_max], decode) =
        decode.split_first_chunk::<4>().map(|(a, b)| (*a, b))?;
    let num_components = decode.len() / 2;

    let mut reader = BitReader::new(data);

    let interpolate_coord = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_cord.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let interpolate_comp = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_comp.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let read_single = |reader: &mut BitReader| -> Option<TriangleVertex> {
        let flag = reader.read(bpf)?;
        let x = interpolate_coord(reader.read(bp_cord)?, x_min, x_max);
        let y = interpolate_coord(reader.read(bp_cord)?, y_min, y_max);

        let mut colors = smallvec![];

        if has_function {
            // Just read the parametric value.
            colors.push(interpolate_comp(
                reader.read(bp_comp)?,
                decode[0],
                decode[1],
            ));
        } else {
            for (_, decode) in (0..num_components).zip(decode.chunks_exact(2)) {
                colors.push(interpolate_comp(
                    reader.read(bp_comp)?,
                    decode[0],
                    decode[1],
                ));
            }
        }

        reader.align();

        let point = Point::new(x as f64, y as f64);

        Some(TriangleVertex {
            flag,
            point,
            colors,
        })
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
            return None;
        }

        triangles.push(Triangle::new(a.clone()?, b.clone()?, c.clone()?));
    }

    Some(triangles)
}

fn read_lattice_triangles(
    data: &[u8],
    bp_cord: u8,
    bp_comp: u8,
    has_function: bool,
    vertices_per_row: u32,
    decode: &[f32],
) -> Option<Vec<Triangle>> {
    let bp_cord = BitSize::from_u8(bp_cord)?;
    let bp_comp = BitSize::from_u8(bp_comp)?;

    let mut lattices = vec![];

    let ([x_min, x_max, y_min, y_max], decode) =
        decode.split_first_chunk::<4>().map(|(a, b)| (*a, b))?;
    let num_components = decode.len() / 2;

    let mut reader = BitReader::new(data);

    let interpolate_coord = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_cord.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let interpolate_comp = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_comp.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let read_single = |reader: &mut BitReader| -> Option<TriangleVertex> {
        let x = interpolate_coord(reader.read(bp_cord)?, x_min, x_max);
        let y = interpolate_coord(reader.read(bp_cord)?, y_min, y_max);

        let mut colors = smallvec![];

        if has_function {
            // Just read the parametric value.
            colors.push(interpolate_comp(
                reader.read(bp_comp)?,
                decode[0],
                decode[1],
            ));
        } else {
            for (_, decode) in (0..num_components).zip(decode.chunks_exact(2)) {
                colors.push(interpolate_comp(
                    reader.read(bp_comp)?,
                    decode[0],
                    decode[1],
                ));
            }
        }

        reader.align();

        let point = Point::new(x as f64, y as f64);

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

    for i in 0..(lattices.len() - 1) {
        for j in 0..(vertices_per_row as usize - 1) {
            triangles.push(Triangle::new(
                lattices[i][j].clone(),
                lattices[i + 1][j].clone(),
                lattices[i][j + 1].clone(),
            ));

            triangles.push(Triangle::new(
                lattices[i + 1][j + 1].clone(),
                lattices[i + 1][j].clone(),
                lattices[i][j + 1].clone(),
            ));
        }
    }

    Some(triangles)
}

fn read_coons_patch_mesh(
    data: &[u8],
    bpf: u8,
    bp_coord: u8,
    bp_comp: u8,
    has_function: bool,
    decode: &[f32],
) -> Option<Vec<CoonsPatch>> {
    let bpf = BitSize::from_u8(bpf)?;
    let bp_coord = BitSize::from_u8(bp_coord)?;
    let bp_comp = BitSize::from_u8(bp_comp)?;

    let ([x_min, x_max, y_min, y_max], decode) =
        decode.split_first_chunk::<4>().map(|(a, b)| (*a, b))?;
    let num_components = decode.len() / 2;

    let mut reader = BitReader::new(data);

    let interpolate_coord = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_coord.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let interpolate_comp = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_comp.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    // Helper to read a single color (or t value)
    let read_colors = |reader: &mut BitReader| -> Option<ColorComponents> {
        let mut colors = smallvec![];
        if has_function {
            colors.push(interpolate_comp(
                reader.read(bp_comp)?,
                decode[0],
                decode[1],
            ));
        } else {
            for (_, decode) in (0..num_components).zip(decode.chunks_exact(2)) {
                colors.push(interpolate_comp(
                    reader.read(bp_comp)?,
                    decode[0],
                    decode[1],
                ));
            }
        }
        Some(colors)
    };

    // State for implicit control points/colors
    let mut prev_patch: Option<CoonsPatch> = None;
    let mut patches = vec![];

    while let Some(flag) = reader.read(bpf) {
        let mut control_points = [Point::ZERO; 12];
        let mut colors = [smallvec![], smallvec![], smallvec![], smallvec![]];

        match flag {
            0 => {
                for i in 0..12 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                for i in 0..4 {
                    colors[i] = read_colors(&mut reader)?;
                }

                prev_patch = Some(CoonsPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            1 => {
                let prev = prev_patch.as_ref()?;

                control_points[0] = prev.control_points[3];
                control_points[1] = prev.control_points[4];
                control_points[2] = prev.control_points[5];
                control_points[3] = prev.control_points[6];
                colors[0] = prev.colors[1].clone();
                colors[1] = prev.colors[2].clone();
                for i in 4..12 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                for i in 2..4 {
                    colors[i] = read_colors(&mut reader)?;
                }

                prev_patch = Some(CoonsPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            2 => {
                let prev = prev_patch.as_ref()?;
                control_points[0] = prev.control_points[6];
                control_points[1] = prev.control_points[7];
                control_points[2] = prev.control_points[8];
                control_points[3] = prev.control_points[9];
                colors[0] = prev.colors[2].clone();
                colors[1] = prev.colors[3].clone();

                for i in 4..12 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                for i in 2..4 {
                    colors[i] = read_colors(&mut reader)?;
                }

                prev_patch = Some(CoonsPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            3 => {
                let prev = prev_patch.as_ref()?;
                control_points[0] = prev.control_points[9];
                control_points[1] = prev.control_points[10];
                control_points[2] = prev.control_points[11];
                control_points[3] = prev.control_points[0];
                colors[0] = prev.colors[3].clone();
                colors[1] = prev.colors[0].clone();

                for i in 4..12 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                for i in 2..4 {
                    colors[i] = read_colors(&mut reader)?;
                }

                prev_patch = Some(CoonsPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            _ => break,
        }

        patches.push(CoonsPatch {
            control_points,
            colors,
        });
    }
    Some(patches)
}

fn read_tensor_product_patch_mesh(
    data: &[u8],
    bpf: u8,
    bp_coord: u8,
    bp_comp: u8,
    has_function: bool,
    decode: &[f32],
) -> Option<Vec<TensorProductPatch>> {
    let bpf = BitSize::from_u8(bpf)?;
    let bp_coord = BitSize::from_u8(bp_coord)?;
    let bp_comp = BitSize::from_u8(bp_comp)?;

    let ([x_min, x_max, y_min, y_max], decode) =
        decode.split_first_chunk::<4>().map(|(a, b)| (*a, b))?;
    let num_components = decode.len() / 2;

    let mut reader = BitReader::new(data);

    let interpolate_coord = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_coord.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    let interpolate_comp = |n: u32, d_min: f32, d_max: f32| {
        interpolate(
            n as f32,
            0.0,
            2.0f32.powi(bp_comp.bits() as i32) - 1.0,
            d_min,
            d_max,
        )
    };

    // Helper to read a single color (or t value)
    let read_colors = |reader: &mut BitReader| -> Option<ColorComponents> {
        let mut colors = smallvec![];
        if has_function {
            colors.push(interpolate_comp(
                reader.read(bp_comp)?,
                decode[0],
                decode[1],
            ));
        } else {
            for (_, decode) in (0..num_components).zip(decode.chunks_exact(2)) {
                colors.push(interpolate_comp(
                    reader.read(bp_comp)?,
                    decode[0],
                    decode[1],
                ));
            }
        }
        Some(colors)
    };

    // State for implicit control points/colors
    let mut prev_patch: Option<TensorProductPatch> = None;
    let mut patches = vec![];

    while let Some(flag) = reader.read(bpf) {
        let mut control_points = [Point::ZERO; 16];
        let mut colors = [smallvec![], smallvec![], smallvec![], smallvec![]];

        match flag {
            0 => {
                for i in 0..16 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                for i in 0..4 {
                    colors[i] = read_colors(&mut reader)?;
                }

                prev_patch = Some(TensorProductPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            1 => {
                let prev = prev_patch.as_ref()?;

                control_points[0] = prev.control_points[3];
                control_points[1] = prev.control_points[4];
                control_points[2] = prev.control_points[5];
                control_points[3] = prev.control_points[6];
                colors[0] = prev.colors[1].clone();
                colors[1] = prev.colors[2].clone();

                for i in 4..16 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                // Read colors for new top-right and bottom-right corners
                colors[2] = read_colors(&mut reader)?; // top-right  
                colors[3] = read_colors(&mut reader)?; // bottom-right

                prev_patch = Some(TensorProductPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            2 => {
                // Bottom edge sharing - previous patch's bottom edge becomes new patch's top edge
                let prev = prev_patch.as_ref()?;

                // For 4x4 grid: bottom edge is [12, 13, 14, 15], top edge is [0, 1, 2, 3]
                control_points[0] = prev.control_points[6]; // bottom-left -> top-left
                control_points[1] = prev.control_points[7]; // -> top edge
                control_points[2] = prev.control_points[8]; // -> top edge
                control_points[3] = prev.control_points[9]; // bottom-right -> top-right
                colors[0] = prev.colors[2].clone(); // prev bottom-left -> new top-left
                colors[1] = prev.colors[3].clone(); // prev bottom-right -> new top-right

                for i in 4..16 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                // Read colors for new bottom corners
                colors[2] = read_colors(&mut reader)?; // bottom-left
                colors[3] = read_colors(&mut reader)?; // bottom-right

                prev_patch = Some(TensorProductPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            3 => {
                // Left edge sharing - previous patch's left edge becomes new patch's right edge
                let prev = prev_patch.as_ref()?;

                // For 4x4 grid: left edge is [0, 4, 8, 12], right edge is [3, 7, 11, 15]
                control_points[0] = prev.control_points[9]; // bottom-left -> top-left
                control_points[1] = prev.control_points[10]; // -> top edge
                control_points[2] = prev.control_points[11]; // -> top edge
                control_points[3] = prev.control_points[0]; // bottom-right -> top-right
                colors[0] = prev.colors[3].clone(); // prev bottom-left -> new top-left
                colors[1] = prev.colors[0].clone(); // prev bottom-right -> new top-right

                for i in 4..16 {
                    let x = interpolate_coord(reader.read(bp_coord)?, x_min, x_max);
                    let y = interpolate_coord(reader.read(bp_coord)?, y_min, y_max);
                    control_points[i] = Point::new(x as f64, y as f64);
                }

                // Read colors for new left corners
                colors[2] = read_colors(&mut reader)?; // top-left
                colors[3] = read_colors(&mut reader)?; // bottom-left

                prev_patch = Some(TensorProductPatch {
                    control_points,
                    colors: colors.clone(),
                });
            }
            _ => break,
        }

        patches.push(TensorProductPatch {
            control_points,
            colors,
        });
    }
    Some(patches)
}

fn read_function(dict: &Dict, color_space: &ColorSpace) -> Option<ShadingFunction> {
    if let Some(arr) = dict.get::<Array>(FUNCTION) {
        let arr: Option<SmallVec<_>> = arr.iter::<Object>().map(|o| Function::new(&o)).collect();
        let arr = arr?;

        if arr.len() != color_space.num_components() as usize {
            warn!("function array of shading has wrong size");

            return None;
        }

        Some(ShadingFunction::Multiple(arr))
    } else if let Some(obj) = dict.get::<Object>(FUNCTION) {
        Some(ShadingFunction::Single(Function::new(&obj)?))
    } else {
        None
    }
}
