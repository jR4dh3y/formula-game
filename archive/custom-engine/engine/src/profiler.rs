use crate::graph::GraphIssue;
use crate::renderer::{DebugView, PassReport, RenderOutput, RenderWarning};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameBudget {
    pub total_ms: f32,
    pub visibility_ms: f32,
    pub prepass_ms: f32,
    pub geometry_ms: f32,
    pub shadows_ms: f32,
    pub reflections_ms: f32,
    pub post_ms: f32,
    pub aa_ms: f32,
}

impl Default for FrameBudget {
    fn default() -> Self {
        Self {
            total_ms: 16.67,
            visibility_ms: 1.0,
            prepass_ms: 0.5,
            geometry_ms: 3.0,
            shadows_ms: 2.0,
            reflections_ms: 2.0,
            post_ms: 1.0,
            aa_ms: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GateFailure {
    TotalFrameBudgetExceeded {
        actual_ms: f32,
        budget_ms: f32,
    },
    PassBudgetExceeded {
        pass: String,
        actual_ms: f32,
        budget_ms: f32,
    },
    SsrSlowerThanSoftwareRt {
        ssr_ms: f32,
        software_rt_ms: f32,
    },
    MissingRequiredDebugView {
        name: &'static str,
    },
    EmptySmaaEdges,
    SuspiciousGraphClear {
        pass: String,
        resource: String,
    },
    RequiredVendorFeature {
        pass: String,
    },
    RequiredTemporalSmoothing {
        pass: String,
    },
    AssetImportFailure {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugViewRequirement {
    pub name: &'static str,
    pub implemented_view: Option<DebugView>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameReport {
    pub total_cpu_ms: f32,
    pub pass_count: usize,
    pub graph_passes: usize,
    pub graph_resources: usize,
    pub failures: Vec<GateFailure>,
}

impl FrameReport {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineProfiler {
    pub budget: FrameBudget,
    pub enforce_timing: bool,
    rendered_debug_views: Vec<DebugView>,
}

impl EngineProfiler {
    pub fn with_budget(budget: FrameBudget) -> Self {
        Self {
            budget,
            rendered_debug_views: Vec::new(),
            enforce_timing: true,
        }
    }

    pub fn mark_debug_view_rendered(&mut self, view: DebugView) {
        if !self.rendered_debug_views.contains(&view) {
            self.rendered_debug_views.push(view);
        }
    }

    pub fn clear_debug_coverage(&mut self) {
        self.rendered_debug_views.clear();
    }

    pub fn evaluate(&self, output: &RenderOutput) -> FrameReport {
        let total_cpu_ms: f32 = output.pass_reports.iter().map(|p| p.cpu_ms).sum();
        let mut failures = Vec::new();
        if self.enforce_timing {
            if total_cpu_ms > self.budget.total_ms {
                failures.push(GateFailure::TotalFrameBudgetExceeded {
                    actual_ms: total_cpu_ms,
                    budget_ms: self.budget.total_ms,
                });
            }
            for pass in &output.pass_reports {
                if pass.cpu_ms > pass.budget_ms {
                    failures.push(GateFailure::PassBudgetExceeded {
                        pass: pass.name.to_string(),
                        actual_ms: pass.cpu_ms,
                        budget_ms: pass.budget_ms,
                    });
                }
                self.evaluate_named_budget(pass, &mut failures);
            }
        }
        for warning in &output.warnings {
            match warning {
                RenderWarning::SsrCostExceededSoftwareRt {
                    ssr_ms,
                    software_rt_ms,
                } if self.enforce_timing => {
                    failures.push(GateFailure::SsrSlowerThanSoftwareRt {
                        ssr_ms: *ssr_ms,
                        software_rt_ms: *software_rt_ms,
                    });
                }
                RenderWarning::SsrCostExceededSoftwareRt { .. } => {}
                RenderWarning::EmptySmaaEdges => failures.push(GateFailure::EmptySmaaEdges),
                RenderWarning::Graph(GraphIssue::SuspiciousClear { pass, resource, .. }) => {
                    failures.push(GateFailure::SuspiciousGraphClear {
                        pass: pass.clone(),
                        resource: resource.clone(),
                    });
                }
                RenderWarning::Graph(GraphIssue::RequiredVendorFeature { pass }) => {
                    failures.push(GateFailure::RequiredVendorFeature { pass: pass.clone() });
                }
                RenderWarning::Graph(GraphIssue::RequiredTemporalSmoothing { pass }) => {
                    failures.push(GateFailure::RequiredTemporalSmoothing { pass: pass.clone() });
                }
                RenderWarning::Asset(issue) => {
                    failures.push(GateFailure::AssetImportFailure {
                        message: format!("{issue:?}"),
                    });
                }
                _ => {}
            }
        }
        FrameReport {
            total_cpu_ms,
            pass_count: output.pass_reports.len(),
            graph_passes: output.graph_passes,
            graph_resources: output.graph_resources,
            failures,
        }
    }

    pub fn evaluate_debug_coverage(&self) -> Vec<GateFailure> {
        required_debug_views()
            .into_iter()
            .filter_map(|req| match req.implemented_view {
                Some(view) if self.rendered_debug_views.contains(&view) => None,
                _ => Some(GateFailure::MissingRequiredDebugView { name: req.name }),
            })
            .collect()
    }

    fn evaluate_named_budget(&self, pass: &PassReport, failures: &mut Vec<GateFailure>) {
        let named_budget = if pass.name.contains("prepass") {
            Some(self.budget.prepass_ms)
        } else if pass.name.contains("base-pass") || pass.name.contains("geometry") {
            Some(self.budget.geometry_ms)
        } else if pass.name.contains("shadow") {
            Some(self.budget.shadows_ms)
        } else if pass.name.contains("reflection")
            || pass.name.contains("rt")
            || pass.name.contains("ssr")
        {
            Some(self.budget.reflections_ms)
        } else if pass.name.contains("fog")
            || pass.name.contains("bloom")
            || pass.name.contains("tonemap")
        {
            Some(self.budget.post_ms)
        } else if pass.name.contains("smaa") || pass.name.contains("persistence") {
            Some(self.budget.aa_ms)
        } else if pass.name.contains("hiz") || pass.name.contains("clear") {
            Some(self.budget.visibility_ms)
        } else {
            None
        };
        if let Some(budget_ms) = named_budget {
            if pass.cpu_ms > budget_ms {
                failures.push(GateFailure::PassBudgetExceeded {
                    pass: pass.name.to_string(),
                    actual_ms: pass.cpu_ms,
                    budget_ms,
                });
            }
        }
    }
}

pub fn required_debug_views() -> Vec<DebugViewRequirement> {
    vec![
        DebugViewRequirement {
            name: "depth-linear-and-hardware",
            implemented_view: Some(DebugView::Depth),
        },
        DebugViewRequirement {
            name: "normals",
            implemented_view: Some(DebugView::Normals),
        },
        DebugViewRequirement {
            name: "roughness",
            implemented_view: Some(DebugView::Roughness),
        },
        DebugViewRequirement {
            name: "material-ids",
            implemented_view: Some(DebugView::MaterialId),
        },
        DebugViewRequirement {
            name: "velocity",
            implemented_view: Some(DebugView::MotionVectors),
        },
        DebugViewRequirement {
            name: "shadow-mask",
            implemented_view: Some(DebugView::ShadowMask),
        },
        DebugViewRequirement {
            name: "ambient-occlusion",
            implemented_view: Some(DebugView::AmbientOcclusion),
        },
        DebugViewRequirement {
            name: "ssr-and-rt-reflections",
            implemented_view: Some(DebugView::Reflections),
        },
        DebugViewRequirement {
            name: "msaa-edge-stencil",
            implemented_view: Some(DebugView::EdgeStencil),
        },
        DebugViewRequirement {
            name: "overdraw-heatmap",
            implemented_view: Some(DebugView::Overdraw),
        },
        DebugViewRequirement {
            name: "texture-count-per-draw",
            implemented_view: Some(DebugView::TextureCount),
        },
        DebugViewRequirement {
            name: "bent-normal-occlusion",
            implemented_view: Some(DebugView::BentNormalOcclusion),
        },
        DebugViewRequirement {
            name: "quad-utilization",
            implemented_view: Some(DebugView::QuadUtilization),
        },
        DebugViewRequirement {
            name: "shadow-caster-participation",
            implemented_view: Some(DebugView::ShadowCasters),
        },
        DebugViewRequirement {
            name: "light-volumes",
            implemented_view: Some(DebugView::LightVolumes),
        },
        DebugViewRequirement {
            name: "temporal-history-validity",
            implemented_view: Some(DebugView::HistoryValidity),
        },
    ]
}
