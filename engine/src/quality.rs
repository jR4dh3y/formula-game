#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityTier {
    Low,
    Medium,
    High,
}

impl QualityTier {
    pub const fn material_texture_limit(self) -> usize {
        match self {
            Self::Low => 2,
            Self::Medium => 3,
            Self::High => 4,
        }
    }

    pub const fn software_rt_resolution_percent(self) -> u32 {
        match self {
            Self::Low => 50,
            Self::Medium => 60,
            Self::High => 70,
        }
    }

    pub const fn temporal_accumulation_frames(self) -> u32 {
        match self {
            Self::Low => 4,
            Self::Medium => 3,
            Self::High => 2,
        }
    }
}
