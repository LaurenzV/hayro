#[derive(Default, Copy, Clone)]
pub(crate) struct ProgressionData {
    layer_num: u16,
    resolution: u8,
    component: u8,
    precinct: u32,
}

struct MaxData {
    layers: u16,
    resolutions: u8,
    components: u8,
    precincts: u32,
}

impl MaxData {
    fn is_max_layer(&self, layer: u16) -> bool {
        layer >= self.layers
    }

    fn is_max_resolution(&self, resolution: u8) -> bool {
        resolution >= self.resolutions
    }

    fn is_max_component(&self, component: u8) -> bool {
        component >= self.components
    }

    fn is_max_precinct(&self, precinct: u32) -> bool {
        precinct >= self.precincts
    }
}

pub(crate) trait ProgressionIterator: Iterator<Item = ProgressionData> {
    fn new(layers: u16, resolutions: u8, components: u8, precincts: u32) -> Self;
}

pub(crate) struct RlcpProgression {
    data: ProgressionData,
    max_data: MaxData,
}

impl Iterator for RlcpProgression {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        if self.max_data.is_max_precinct(self.data.precinct) {
            self.data.precinct = 0;
            self.data.component += 1;
        }

        if self.max_data.is_max_component(self.data.component) {
            self.data.component = 0;
            self.data.layer_num += 1;
        }

        if self.max_data.is_max_layer(self.data.layer_num) {
            self.data.layer_num = 0;
            self.data.resolution += 1;
        }

        if self.max_data.is_max_resolution(self.data.resolution) {
            return None;
        }

        Some(self.data)
    }
}

impl ProgressionIterator for RlcpProgression {
    fn new(layers: u16, resolutions: u8, components: u8, precincts: u32) -> Self {
        Self {
            data: Default::default(),
            max_data: MaxData {
                layers,
                resolutions,
                components,
                precincts,
            },
        }
    }
}
