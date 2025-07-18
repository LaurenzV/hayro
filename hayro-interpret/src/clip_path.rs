use crate::FillRule;
use kurbo::BezPath;

#[derive(Debug, Clone)]
pub struct ClipPath {
    pub path: BezPath,
    pub fill: FillRule,
}
