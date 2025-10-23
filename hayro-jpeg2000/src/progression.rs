#[derive(Default)]
pub(crate) struct ProgressionData {
    layer_num: u16,
    resolution: u8,
    component: u8,
    precinct: u32
}

struct MaxData {
    layers: u16, 
    resolutions: u8, 
    components: u8, 
    precincts: u32
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

trait ProgressionIterator: Iterator<Item = ProgressionData> {
    fn new(layers: u16, num_resolutions: u8, num_components: u8, precincts: u32) -> Self;
}

pub(crate) struct LrcpProgressor {
    data: ProgressionData, 
    max_data: MaxData,
}

impl Iterator for LrcpProgressor {
    type Item = ProgressionData;

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}