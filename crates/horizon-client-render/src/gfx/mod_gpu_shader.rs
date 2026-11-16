use super::native_post_shader::static_cost;

pub const MAX_SHADER_BYTES: usize = 256 * 1024;
pub const LOOP_ITERATIONS: u32 = 256;
pub const ENGINE_GROUP: u32 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl From<String> for ValidationError {
    fn from(message: String) -> Self {
        Self {
            message,
            line: 0,
            column: 0,
        }
    }
}

pub struct ValidatedShader {
    module: naga::Module,
    pub engine_masks: std::collections::BTreeMap<String, u32>,
    pub costs: std::collections::BTreeMap<String, u64>,
}

impl ValidatedShader {
    pub fn into_module(self) -> naga::Module {
        self.module
    }
}

fn guard_block(
    block: naga::Block,
    function: &mut naga::Function,
    counter_type: naga::Handle<naga::Type>,
) -> Result<naga::Block, String> {
    let mut guarded = naga::Block::new();
    for (mut statement, span) in block.span_into_iter() {
        match &mut statement {
            naga::Statement::Block(block) => {
                *block = guard_block(std::mem::take(block), function, counter_type)?;
            }
            naga::Statement::If { accept, reject, .. } => {
                *accept = guard_block(std::mem::take(accept), function, counter_type)?;
                *reject = guard_block(std::mem::take(reject), function, counter_type)?;
            }
            naga::Statement::Switch { cases, .. } => {
                for case in cases {
                    case.body =
                        guard_block(std::mem::take(&mut case.body), function, counter_type)?;
                }
            }
            naga::Statement::Loop {
                body, continuing, ..
            } => {
                *body = guard_block(std::mem::take(body), function, counter_type)?;
                *continuing = guard_block(std::mem::take(continuing), function, counter_type)?;
                let zero = function
                    .expressions
                    .append(naga::Expression::Literal(naga::Literal::U32(0)), span);
                let one = function
                    .expressions
                    .append(naga::Expression::Literal(naga::Literal::U32(1)), span);
                let limit = function.expressions.append(
                    naga::Expression::Literal(naga::Literal::U32(LOOP_ITERATIONS)),
                    span,
                );
                let local = function.local_variables.append(
                    naga::LocalVariable {
                        name: None,
                        ty: counter_type,
                        init: Some(zero),
                    },
                    span,
                );
                let pointer = function
                    .expressions
                    .append(naga::Expression::LocalVariable(local), span);
                let value = function
                    .expressions
                    .append(naga::Expression::Load { pointer }, span);
                let condition = function.expressions.append(
                    naga::Expression::Binary {
                        op: naga::BinaryOperator::GreaterEqual,
                        left: value,
                        right: limit,
                    },
                    span,
                );
                let increment = function.expressions.append(
                    naga::Expression::Binary {
                        op: naga::BinaryOperator::Add,
                        left: value,
                        right: one,
                    },
                    span,
                );
                guarded.push(
                    naga::Statement::Store {
                        pointer,
                        value: zero,
                    },
                    span,
                );
                let mut prefix = naga::Block::new();
                prefix.push(
                    naga::Statement::Emit(naga::Range::new_from_bounds(value, condition)),
                    span,
                );
                let mut stop = naga::Block::new();
                stop.push(naga::Statement::Break, span);
                prefix.push(
                    naga::Statement::If {
                        condition,
                        accept: stop,
                        reject: naga::Block::new(),
                    },
                    span,
                );
                prefix.push(
                    naga::Statement::Emit(naga::Range::new_from_bounds(increment, increment)),
                    span,
                );
                prefix.push(
                    naga::Statement::Store {
                        pointer,
                        value: increment,
                    },
                    span,
                );
                prefix.extend_block(std::mem::take(body));
                *body = prefix;
            }
            naga::Statement::ControlBarrier(_)
            | naga::Statement::MemoryBarrier(_)
            | naga::Statement::WorkGroupUniformLoad { .. }
            | naga::Statement::RayQuery { .. }
            | naga::Statement::SubgroupBallot { .. }
            | naga::Statement::SubgroupGather { .. }
            | naga::Statement::SubgroupCollectiveOperation { .. } => {
                return Err("barriers, ray queries and subgroup operations are unavailable in guarded shaders".into());
            }
            _ => {}
        }
        guarded.push(statement, span);
    }
    Ok(guarded)
}

fn engine_bindings(module: &naga::Module) -> Result<(), String> {
    for (_, variable) in module.global_variables.iter() {
        if variable.space == naga::AddressSpace::WorkGroup {
            return Err("workgroup memory is unavailable in guarded shaders".into());
        }
        let Some(binding) = &variable.binding else {
            continue;
        };
        if binding.group > ENGINE_GROUP {
            return Err("bind group index exceeds 3".into());
        }
        if binding.group != ENGINE_GROUP {
            continue;
        }
        let allowed = match (&module.types[variable.ty].inner, binding.binding) {
            (_, 0) => variable.space == naga::AddressSpace::Uniform,
            (naga::TypeInner::Sampler { comparison: false }, 2) => true,
            (
                naga::TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed,
                    class:
                        naga::ImageClass::Sampled {
                            kind: naga::ScalarKind::Float,
                            multi: false,
                        },
                },
                1 | 4 | 5,
            ) => *arrayed == (binding.binding == 1),
            (
                naga::TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed: false,
                    class: naga::ImageClass::Depth { multi: false },
                },
                3,
            ) => true,
            _ => false,
        };
        if !allowed {
            return Err("engine resources must use the reserved read-only binding types".into());
        }
    }
    Ok(())
}

pub fn validate(source: &str) -> Result<ValidatedShader, ValidationError> {
    if source.is_empty() || source.len() > MAX_SHADER_BYTES {
        return Err("shader source must contain 1 through 262144 bytes"
            .to_owned()
            .into());
    }
    let mut module = naga::front::wgsl::parse_str(source).map_err(|error| {
        let location = error.location(source);
        ValidationError {
            message: error.emit_to_string(source),
            line: location.map_or(0, |location| location.line_number),
            column: location.map_or(0, |location| location.line_position),
        }
    })?;
    if !module.overrides.is_empty() {
        return Err("pipeline overrides are unavailable in guarded shaders"
            .to_owned()
            .into());
    }
    let functions = module
        .functions
        .iter()
        .map(|(_, function)| function)
        .chain(module.entry_points.iter().map(|entry| &entry.function));
    if module.functions.len() > 64
        || module.entry_points.len() > 16
        || functions
            .into_iter()
            .any(|function| function.expressions.len() > 4096)
    {
        return Err(
            "shader function or expression count exceeds the sandbox limit"
                .to_owned()
                .into(),
        );
    }
    engine_bindings(&module)?;
    let counter_type = module.types.insert(
        naga::Type {
            name: None,
            inner: naga::TypeInner::Scalar(naga::Scalar::U32),
        },
        naga::Span::UNDEFINED,
    );
    for (_, function) in module.functions.iter_mut() {
        function.body = guard_block(std::mem::take(&mut function.body), function, counter_type)?;
    }
    for entry in &mut module.entry_points {
        entry.function.body = guard_block(
            std::mem::take(&mut entry.function.body),
            &mut entry.function,
            counter_type,
        )?;
        if entry.stage == naga::ShaderStage::Compute
            && entry
                .workgroup_size
                .into_iter()
                .try_fold(1_u32, u32::checked_mul)
                .is_none_or(|size| size == 0 || size > 256)
        {
            return Err("compute workgroup size exceeds 256 invocations"
                .to_owned()
                .into());
        }
    }
    let information = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|error| {
        let location = error.location(source);
        ValidationError {
            message: error.emit_to_string(source),
            line: location.map_or(0, |location| location.line_number),
            column: location.map_or(0, |location| location.line_position),
        }
    })?;
    let mut layout = naga::proc::Layouter::default();
    layout
        .update(module.to_ctx())
        .map_err(|error| error.to_string())?;
    if module
        .types
        .iter()
        .any(|(handle, _)| layout[handle].size > 16384)
        || module
            .global_variables
            .iter()
            .filter(|(_, variable)| variable.space == naga::AddressSpace::Private)
            .map(|(_, variable)| u64::from(layout[variable.ty].size))
            .sum::<u64>()
            > 16384
    {
        return Err("shader type size or private memory exceeds 16384 bytes"
            .to_owned()
            .into());
    }
    let local_bytes = module
        .functions
        .iter()
        .map(|(_, function)| function)
        .chain(module.entry_points.iter().map(|entry| &entry.function))
        .flat_map(|function| {
            function
                .local_variables
                .iter()
                .map(|(_, variable)| variable)
        })
        .map(|variable| u64::from(layout[variable.ty].size))
        .sum::<u64>();
    if local_bytes > 16384 {
        return Err("shader function locals exceed 16384 bytes"
            .to_owned()
            .into());
    }
    let costs = module
        .entry_points
        .iter()
        .map(|entry| {
            let invocations = entry
                .workgroup_size
                .into_iter()
                .map(u64::from)
                .product::<u64>()
                .max(1);
            static_cost(&module, &entry.function, LOOP_ITERATIONS)
                .map(|cost| (entry.name.clone(), cost.saturating_mul(invocations)))
        })
        .collect::<Result<_, _>>()?;
    let engine_masks = module
        .entry_points
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let usage = information.get_entry_point(index);
            let mask = module
                .global_variables
                .iter()
                .filter_map(|(handle, variable)| {
                    let binding = variable.binding.as_ref()?;
                    (binding.group == ENGINE_GROUP && !usage[handle].is_empty())
                        .then_some(1 << binding.binding)
                })
                .fold(0, |mask, binding| mask | binding);
            (entry.name.clone(), mask)
        })
        .collect();
    Ok(ValidatedShader {
        module,
        engine_masks,
        costs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(source: &str) -> ValidationError {
        validate(source).err().expect("shader must be rejected")
    }

    #[test]
    fn bounds_source_and_reports_locations() {
        assert!(
            rejected(&" ".repeat(MAX_SHADER_BYTES + 1))
                .message
                .contains("262144")
        );
        assert!(rejected("").message.contains("262144"));
        let error = rejected("@compute @workgroup_size(1)\nfn main() { invalid; }");
        assert_eq!(error.line, 2);
        assert!(error.column > 0);
    }

    #[test]
    fn guards_infinite_loops_and_continues_in_helpers() {
        validate("@compute @workgroup_size(1) fn main() { loop {} }").unwrap();
        validate("fn helper() { loop { continue; } } @compute @workgroup_size(1) fn main() { helper(); }").unwrap();
        validate("@compute @workgroup_size(1) fn main() { for (var index = 0u; index < 2u; index++) {} }").unwrap();
    }

    #[test]
    fn rejects_engine_writes_and_unguarded_collectives() {
        assert!(rejected("@group(3) @binding(0) var<storage, read_write> data: array<u32>; @compute @workgroup_size(1) fn main() { data[0] = 1u; }").message.contains("read-only"));
        assert!(rejected("@group(3) @binding(4) var image: texture_storage_2d<rgba8unorm, write>; @compute @workgroup_size(1) fn main() {}").message.contains("read-only"));
        assert!(
            rejected("@compute @workgroup_size(1) fn main() { workgroupBarrier(); }")
                .message
                .contains("barrier")
        );
        assert!(
            rejected("var<workgroup> data: u32; @compute @workgroup_size(1) fn main() {}")
                .message
                .contains("workgroup memory")
        );
        assert!(
            rejected("@compute @workgroup_size(257) fn main() {}")
                .message
                .contains("256 invocations")
        );
        assert!(
            rejected("override workers: u32 = 1u; @compute @workgroup_size(workers) fn main() {}")
                .message
                .contains("pipeline overrides")
        );
    }

    #[test]
    fn function_locals_have_an_aggregate_storage_limit() {
        let error = rejected(
            "fn helper() { var first: array<u32, 3072>; first[0] = 1u; } @compute @workgroup_size(1) fn main() { var second: array<u32, 3072>; second[0] = 1u; helper(); }",
        );
        assert!(error.message.contains("locals"), "{}", error.message);
    }

    #[test]
    fn bounded_loops_still_have_call_graph_cost_limits() {
        assert!(
            rejected("@compute @workgroup_size(1) fn main() { loop { loop {} } }")
                .message
                .contains("expression-equivalents")
        );
    }
}
