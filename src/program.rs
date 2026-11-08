//! Shading programs: user WGSL `surface` and `shade` functions composed with the engine
//! prelude, the program API and a generated entry point into one module per stage and
//! visibility variant. naga checks each program before the device compiles it, so
//! diagnostics name program lines, and `struct Parameters` is reflected from naga IR.
//! See docs/programmable-shading.md.
use crate::{Error, observe::Owned};
use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

/// The built-in photoreal program every material uses unless it names another.
pub const STANDARD_PROGRAM: &str = include_str!("../shaders/programs/standard.wgsl");
const SPECULAR_API: &str = include_str!("../shaders/specular_filter.wgsl");
const COMMON_API: &str = include_str!("../shaders/program_api.wgsl");
// DC1: resolved decal inputs are available to every user program on every stage.
const DECAL_API: &str = include_str!("../shaders/decals.wgsl");
const RASTER_API: &str = include_str!("../shaders/program_raster.wgsl");
const RASTER_ENTRY: &str = include_str!("../shaders/program_raster_entry.wgsl");
const TRACED_API: &str = include_str!("../shaders/program_traced.wgsl");
const TRACED_ENTRY: &str = include_str!("../shaders/program_traced_entry.wgsl");
/// The traced `sample_*_grad` helpers of frames that stream textures, which also record
/// residency requests (texture_residency.rs). They replace the traced API's own.
const TRACED_RESIDENCY: &str = include_str!("../shaders/program_traced_residency.wgsl");
/// Raster materials bind their block as a uniform beside the fixed material ABI.
const RASTER_PARAMETERS: &str = "@group(2) @binding(7) var<uniform> params:Parameters;\n";
/// Traced shading reads each draw's block from a binding array and copies the hit's.
const TRACED_PARAMETERS: &str = "struct RlParameters { value:Parameters };\n@group(2) @binding(17) var<storage,read> rl_parameters:binding_array<RlParameters,PRIMARY_BATCHES>;\nvar<private> params:Parameters;\nfn rl_load_parameters(slot:u32) { params=rl_parameters[slot].value; }\n";
const TRACED_NO_PARAMETERS: &str = "fn rl_load_parameters(slot:u32) {}\n";
/// M7 indirect-light inputs (world radiance cache, then the occluded environment specular
/// of M7-ES1): composed only for programs that call them, so every other program's modules
/// are unchanged (shaders/gi_api.wgsl, shaders/environment_lobes.wgsl).
const INDIRECT_API: [&str; 3] = [
    include_str!("../shaders/gi_cells.wgsl"),
    include_str!("../shaders/gi_api.wgsl"),
    include_str!("../shaders/environment_lobes.wgsl"),
];
/// The world radiance cache's cell shading entry (M7 G2): a program's `surface` plus its
/// optional `scatter`/`emission` at each cell's query point (shaders/gi_shade_entry.wgsl).
const CELL_ENTRY: &str = include_str!("../shaders/gi_shade_entry.wgsl");
/// The M7-ES1 inputs that read the receiver's sky visibility for specular. The generated
/// entry prepares the receiver's one cache lookup before `surface` for every program that
/// calls an indirect input (`rl_indirect_prepare`: a lookup at each call site cost 0.4 ms of
/// shading on the quarry), and the specular visibility from it for programs that call one
/// of these.
const SPECULAR_INPUTS: [&str; 4] = [
    "indirect_radiance",
    "environment_lobe_count",
    "environment_lobe",
    "environment_lobe_reflection",
];
/// The engine inputs `INDIRECT_API` defines.
const INDIRECT_INPUTS: [&str; 5] = [
    "indirect_irradiance",
    "indirect_radiance",
    "environment_lobe_count",
    "environment_lobe",
    "environment_lobe_reflection",
];

// ---- M7-R1 reflections (docs/m7-r1.md) ----
/// Traced reflections for programs that declare the optional
/// `fn reflection_lobe(s: Surface, input: SurfaceInput) -> SpecularLobe` and read
/// `specular_incident(input) -> vec4<f32>` (shaders/reflections.wgsl). `RL_SCENE` names the
/// head's shadow TLAS.
const REFLECTION_API: &str = include_str!("../shaders/reflections.wgsl");
/// Every other composition of such a program: raster, cell and non-RT traced stages, and
/// programs that use only one half of the pair. Its untraced estimate stays the answer.
const REFLECTION_STUB: &str = "struct SpecularLobe { normal:vec3<f32>, alpha:f32, weight:f32 };\nfn specular_incident(input:SurfaceInput)->vec4<f32> { return vec4<f32>(0.0); }\n";
/// The traced entry traces the lobe after the receiver's `surface`, before `shade` reads it.
/// The receiver's indirect inputs are prepared after the trace, so hit shading never
/// overwrites them and the cache's prepared state is not live across the ray loop (a restore
/// after the loop cost +0.62 ms of quarry shading with the cache on). `surface` and the hits
/// read the open environment state (`rl_indirect_open`).
const REFLECTION_ENTRY_SEAM: &str =
    "    rl_indirect_prepare(surface_input);\n    let surface_output=surface(surface_input);\n";
const REFLECTION_ENTRY: &str = "    rl_indirect_open();\n    let surface_output=surface(surface_input);\n    rl_reflection_prepare(surface_output,surface_input,hit.batch,id,size);\n    rl_indirect_prepare(surface_input);\n";
/// Reflection hits trace their own light visibility: the frame's belongs to its receivers.
const REFLECTION_VISIBILITY_SEAM: &str =
    "    return hard_shadow(lights[s.index],input.receiver,s.direction);\n";
const REFLECTION_VISIBILITY: &str = "    if rl_reflection_hit {return rl_hit_shadow(lights[s.index],input.receiver,s.direction);}\n";

/// The reflection API text for `program` on `head` at `stage`, and whether it traces: empty
/// for programs that name neither hook nor input, so their modules are unchanged.
fn reflection_api(head: &str, program: &str, stage: Stage) -> (String, bool) {
    let names = tokens(program);
    let lobe = names.windows(2).any(|w| w == ["fn", "reflection_lobe"]);
    let reads = names.contains(&"specular_incident");
    if !lobe && !reads {
        return (String::new(), false);
    }
    // Hardware heads declare their shadow TLAS at binding 2; non-RT heads have none.
    let scene = ["hardware_scene", "fresh_scene"]
        .into_iter()
        .find(|name| head.contains(&format!("var {name}:acceleration_structure;")));
    match scene.filter(|_| lobe && reads && matches!(stage, Stage::Traced | Stage::TracedStreamed))
    {
        Some(scene) => (REFLECTION_API.replace("RL_SCENE", scene), true),
        None => (REFLECTION_STUB.to_owned(), false),
    }
}
// ---- end of M7-R1 reflections ----

/// A world radiance cache shader's source (`name` under shaders/). Test builds read the
/// same-named file from RENDERLAB_GI_SHADERS when it exists, so a lease can screen
/// shader variants on one binary, one process per variant (docs/m7-g1b.md).
pub(crate) fn gi_shader(name: &str, builtin: &'static str) -> std::borrow::Cow<'static, str> {
    #[cfg(test)]
    if let Some(text) = std::env::var_os("RENDERLAB_GI_SHADERS")
        .and_then(|dir| std::fs::read_to_string(std::path::Path::new(&dir).join(name)).ok())
    {
        return text.into();
    }
    let _ = name;
    builtin.into()
}
fn indirect_api() -> String {
    let mut source = gi_shader("gi_cells.wgsl", INDIRECT_API[0]).into_owned();
    source.push_str(&gi_shader("gi_api.wgsl", INDIRECT_API[1]));
    source.push_str(&gi_shader("environment_lobes.wgsl", INDIRECT_API[2]));
    source
}

/// Source of a shading program: WGSL without bindings that defines
/// `fn surface(input: SurfaceInput) -> Surface` and
/// `fn shade(s: Surface, input: SurfaceInput) -> vec3<f32>` (exposed linear HDR), and may
/// declare `struct Parameters` of f32, vec2/3/4<f32>, u32, i32 and fixed arrays of
/// these types, read as `params`. Arrays must satisfy WGSL uniform layout (e.g.
/// `array<vec4<f32>, 2>`). Optional `const DEFAULT_PARAMETERS: Parameters = ...`
/// supplies omitted material values. The API is shaders/program_api.wgsl plus its
/// raster and traced stages.
#[derive(Clone, Copy, Debug)]
pub struct ProgramDescriptor<'a> {
    pub label: &'a str,
    pub source: &'a str,
}

/// Owned handle to one renderer's shading program. Materials retain their program; the
/// renderer owns its pipelines and releases them once every handle has dropped.
#[derive(Clone)]
pub struct Program(pub(crate) Arc<ProgramShared>);
pub(crate) struct ProgramShared {
    pub id: u64,
    pub renderer: Arc<()>,
    pub label: String,
}
impl Program {
    pub fn label(&self) -> &str {
        &self.0.label
    }
    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }
}
impl PartialEq for Program {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Program {}
impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Program({:?})", self.0.label)
    }
}

/// One `Parameters` member's value, checked against the member's name and type.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    F32(f32),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    U32(u32),
    I32(i32),
    /// A fixed array, checked for element type and exact length. For scalar
    /// thresholds, pack four per vec4 to satisfy WGSL's uniform array stride.
    Array(Vec<Value>),
}
impl Value {
    fn matches(&self, kind: &Kind) -> bool {
        match (self, kind) {
            (Self::F32(_), Kind::F32)
            | (Self::Vec2(_), Kind::Vec2)
            | (Self::Vec3(_), Kind::Vec3)
            | (Self::Vec4(_), Kind::Vec4)
            | (Self::U32(_), Kind::U32)
            | (Self::I32(_), Kind::I32) => true,
            (Self::Array(values), Kind::Array { element, len, .. }) => {
                values.len() == *len as usize && values.iter().all(|v| v.matches(element))
            }
            _ => false,
        }
    }
    fn write(&self, kind: &Kind, out: &mut [u8]) {
        if let (
            Self::Array(values),
            Kind::Array {
                element, stride, ..
            },
        ) = (self, kind)
        {
            for (value, dst) in values.iter().zip(out.chunks_mut(*stride as usize)) {
                value.write(element, dst);
            }
            return;
        }
        let words: Vec<[u8; 4]> = match self {
            Self::F32(v) => vec![v.to_ne_bytes()],
            Self::Vec2(v) => v.map(f32::to_ne_bytes).to_vec(),
            Self::Vec3(v) => v.map(f32::to_ne_bytes).to_vec(),
            Self::Vec4(v) => v.map(f32::to_ne_bytes).to_vec(),
            Self::U32(v) => vec![v.to_ne_bytes()],
            Self::I32(v) => vec![v.to_ne_bytes()],
            Self::Array(_) => unreachable!("array type checked before packing"),
        };
        for (dst, word) in out.chunks_exact_mut(4).zip(words) {
            dst.copy_from_slice(&word);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    F32,
    Vec2,
    Vec3,
    Vec4,
    U32,
    I32,
    Array {
        element: Box<Kind>,
        len: u32,
        stride: u32,
    },
}
impl Kind {
    fn size(&self) -> usize {
        match self {
            Self::F32 | Self::U32 | Self::I32 => 4,
            Self::Vec2 => 8,
            Self::Vec3 => 12,
            Self::Vec4 => 16,
            Self::Array { len, stride, .. } => (*len * *stride) as usize,
        }
    }
}

/// Bytes of every material's parameter block; `Parameters` may use at most this many.
pub(crate) const PARAMETER_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    pub name: String,
    pub kind: Kind,
    pub offset: u32,
}

/// A program's reflected `struct Parameters`, in member order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ParameterLayout {
    pub fields: Vec<Field>,
    pub defaults: [u8; PARAMETER_BYTES],
}
impl ParameterLayout {
    /// Every named value must name a member of the same type.
    pub fn check<'a>(
        layout: Option<&Self>,
        values: impl IntoIterator<Item = (&'a str, Value)>,
    ) -> Result<(), Error> {
        for (name, value) in values {
            let field = layout
                .and_then(|l| l.fields.iter().find(|f| f.name == name))
                .ok_or(Error::InvalidInput(
                    "material parameter is not a member of its program's Parameters",
                ))?;
            if !value.matches(&field.kind) {
                return Err(Error::InvalidInput(
                    "material parameter type differs from its Parameters member",
                ));
            }
        }
        Ok(())
    }
    /// Program defaults, then explicit overrides at naga-reflected offsets/strides.
    /// Values naming no member of the same type are skipped.
    pub fn pack(layout: Option<&Self>, values: &[(String, Value)]) -> [u8; PARAMETER_BYTES] {
        let mut block = layout.map_or([0; PARAMETER_BYTES], |l| l.defaults);
        for (name, value) in values {
            if let Some(f) = layout
                .and_then(|l| l.fields.iter().find(|f| f.name == *name))
                .filter(|f| value.matches(&f.kind))
            {
                let offset = f.offset as usize;
                // Explicit arrays also clear any default padding bytes.
                block[offset..offset + f.kind.size()].fill(0);
                value.write(&f.kind, &mut block[offset..offset + f.kind.size()]);
            }
        }
        block
    }
    /// New or retyped members without a nonzero program default.
    pub fn changed(previous: Option<&Self>, next: Option<&Self>) -> Vec<String> {
        let Some(next) = next else {
            return vec![];
        };
        next.fields
            .iter()
            .filter(|f| {
                next.defaults[f.offset as usize..f.offset as usize + f.kind.size()]
                    .iter()
                    .all(|&b| b == 0)
                    && !previous.is_some_and(|p| {
                        p.fields
                            .iter()
                            .any(|o| o.name == f.name && o.kind == f.kind)
                    })
            })
            .map(|f| f.name.clone())
            .collect()
    }
}

/// One material's parameter values and GPU block, repacked by name when its program
/// reloads with a different layout.
pub(crate) struct MaterialParameters {
    pub buffer: Owned<wgpu::Buffer>,
    pub values: Mutex<Vec<(String, Value)>>,
}

/// The stage a composition targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Raster,
    Traced,
    /// Traced, with the residency-recording sampling helpers (`TRACED_RESIDENCY`).
    TracedStreamed,
    /// The world radiance cache's cell shading (M7 G2): the traced material ABI and the
    /// cache definitions with `CELL_ENTRY`, for programs that `reflects`.
    Cell,
}

/// The traced API with its `sample_*_grad` helpers, from the base-colour one to the DFG
/// helper that follows them, replaced by `TRACED_RESIDENCY`. `Stage::Traced` composes
/// the traced API unchanged, so default shading contains no residency code.
fn traced_streamed_api() -> String {
    let start = TRACED_API.find("fn sample_base_color_grad(").unwrap();
    let end = TRACED_API.find("// The DFG table has one level").unwrap();
    format!(
        "{}{TRACED_RESIDENCY}{}",
        &TRACED_API[..start],
        &TRACED_API[end..]
    )
}

/// The traced entry's hit-record helpers (`PrimaryHit`, `primary_decode`, `primary_input`),
/// everything before its `primary_vertex`. A traced head with RT1d's shadow visibility filter
/// calls them, so a cell module composed on that head defines them too; cell shading itself
/// never runs the filter (its lights come from the cache's light pass).
fn traced_hit_helpers() -> &'static str {
    &TRACED_ENTRY[..TRACED_ENTRY.find("fn primary_vertex(").unwrap()]
}

/// A composed module and the bytes its program occupies.
pub(crate) struct Composed {
    pub source: String,
    pub program: Range<usize>,
}

/// One module: `head` (the prelude with its visibility variant and, for traced stages,
/// the ray helpers), the shared API, the stage API, the parameter binding when
/// `parameters`, the program, then the generated entry point.
pub(crate) fn compose(head: &str, program: &str, stage: Stage, parameters: bool) -> Composed {
    let streamed;
    let (api, binding, entry) = match stage {
        Stage::Raster => (
            RASTER_API,
            if parameters { RASTER_PARAMETERS } else { "" },
            RASTER_ENTRY,
        ),
        Stage::Traced | Stage::TracedStreamed | Stage::Cell => (
            if stage == Stage::TracedStreamed {
                streamed = traced_streamed_api();
                &streamed
            } else {
                TRACED_API
            },
            if parameters {
                TRACED_PARAMETERS
            } else {
                TRACED_NO_PARAMETERS
            },
            if stage == Stage::Cell {
                CELL_ENTRY
            } else {
                TRACED_ENTRY
            },
        ),
    };
    let indirect_calls = uses_indirect(program);
    // Cell shading reads the cache's own definitions whatever the program calls.
    let mut indirect = if indirect_calls || stage == Stage::Cell {
        indirect_api()
    } else {
        String::new()
    };
    // Each entry calls this once per receiver before `surface`: the receiver's one cache
    // lookup for programs that call an indirect input, and the specular visibility from it
    // for those that read the specular inputs.
    let specular = tokens(program).iter().any(|t| SPECULAR_INPUTS.contains(t));
    indirect.push_str(match (indirect_calls, specular) {
        (true, true) => {
            "fn rl_indirect_prepare(input:SurfaceInput) { gi_prepare(input); es_prepare(input); }\n"
        }
        (true, false) => "fn rl_indirect_prepare(input:SurfaceInput) { gi_prepare(input); }\n",
        _ => "fn rl_indirect_prepare(input:SurfaceInput) {}\n",
    });
    // LC1: the light loop's clusters and local shadows, traced where the head binds rays.
    let lights = crate::light_cluster::shading_api(
        head,
        matches!(stage, Stage::Traced | Stage::TracedStreamed),
    );
    // AT1: camera-pixel entries apply the sky and fog (shaders/atmosphere_api.wgsl); cell
    // shading's bounce light carries none.
    let atmosphere = if stage == Stage::Cell {
        String::new()
    } else {
        crate::atmosphere::api_source()
    };
    let (mut reflection, traces) = reflection_api(head, program, stage);
    if traces {
        // M7-R1: the indirect inputs' cache-off state, which reads the open environment.
        reflection.push_str(match (indirect_calls, specular) {
            (true, true) => {
                "fn rl_indirect_open() { gi_rx_valid=false; es_visibility=vec3<f32>(1.0); }\n"
            }
            (true, false) => "fn rl_indirect_open() { gi_rx_valid=false; }\n",
            _ => "fn rl_indirect_open() {}\n",
        });
    }
    let mut source = format!(
        "{head}\n{COMMON_API}{lights}{DECAL_API}{SPECULAR_API}{api}{indirect}{reflection}{atmosphere}{binding}// ---- program ----\n"
    );
    if traces {
        // M7-R1: hits trace their own light visibility.
        assert_eq!(source.matches(REFLECTION_VISIBILITY_SEAM).count(), 1);
        source = source.replacen(
            REFLECTION_VISIBILITY_SEAM,
            &format!("{REFLECTION_VISIBILITY}{REFLECTION_VISIBILITY_SEAM}"),
            1,
        );
    }
    let start = source.len();
    source.push_str(program);
    let program_range = start..source.len();
    source.push_str("\n// ---- end of program ----\n");
    if stage == Stage::Cell {
        source.push_str(&reflection_wrappers(program));
        if head.contains("primary_decode(") {
            source.push_str(traced_hit_helpers());
        }
    }
    if traces {
        // M7-R1: the lobe is traced between the receiver's `surface` and `shade`.
        assert_eq!(entry.matches(REFLECTION_ENTRY_SEAM).count(), 1);
        source.push_str(&entry.replacen(REFLECTION_ENTRY_SEAM, REFLECTION_ENTRY, 1));
    } else {
        source.push_str(entry);
    }
    Composed {
        source,
        program: program_range,
    }
}

/// Whether `program` declares the optional bounce-light hooks (M7 G2): `fn scatter(s:
/// Surface, input: SurfaceInput, wi: vec3<f32>, wo: vec3<f32>) -> vec3<f32>`, the radiance it
/// sends toward `wo` per unit radiance arriving from `wi` (its BRDF times the cosine at `wi`),
/// and `fn emission(s: Surface, input: SurfaceInput) -> vec3<f32>`, its own radiance. A program
/// that declares neither still blocks light in the world radiance cache but reflects none.
pub(crate) fn reflects(program: &str) -> bool {
    declares(program, "fn", "scatter") || declares(program, "fn", "emission")
}
/// Cell shading's calls into the hooks, zero for an absent one.
fn reflection_wrappers(program: &str) -> String {
    let scatter = if declares(program, "fn", "scatter") {
        "return scatter(s,input,wi,wo);"
    } else {
        "return vec3<f32>(0.0);"
    };
    let emission = if declares(program, "fn", "emission") {
        "return emission(s,input);"
    } else {
        "return vec3<f32>(0.0);"
    };
    format!(
        "fn rl_scatter(s:Surface,input:SurfaceInput,wi:vec3<f32>,wo:vec3<f32>)->vec3<f32> {{ {scatter} }}\nfn rl_emission(s:Surface,input:SurfaceInput)->vec3<f32> {{ {emission} }}\n"
    )
}

/// The program's identifiers in order, without comments (WGSL block comments nest).
fn tokens<'a>(source: &'a str) -> Vec<&'a str> {
    let bytes = source.as_bytes();
    let (mut out, mut i, mut depth, mut start) = (vec![], 0, 0usize, None);
    let flush = |out: &mut Vec<&'a str>, start: &mut Option<usize>, end: usize| {
        if let Some(s) = start.take() {
            out.push(&source[s..end]);
        }
    };
    while i < bytes.len() {
        let pair = &bytes[i..bytes.len().min(i + 2)];
        if pair == b"/*" {
            flush(&mut out, &mut start, i);
            depth += 1;
            i += 2;
        } else if depth > 0 && pair == b"*/" {
            depth -= 1;
            i += 2;
        } else if depth > 0 {
            i += 1;
        } else if pair == b"//" {
            flush(&mut out, &mut start, i);
            i += source[i..].find('\n').unwrap_or(source.len() - i);
        } else if bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' {
            start.get_or_insert(i);
            i += 1;
        } else {
            flush(&mut out, &mut start, i);
            i += 1;
        }
    }
    flush(&mut out, &mut start, bytes.len());
    out
}
fn declares(source: &str, keyword: &str, name: &str) -> bool {
    tokens(source).windows(2).any(|w| w == [keyword, name])
}
/// Whether `program` names an indirect-light engine input outside comments; those
/// programs get the cache inputs and make frames run the cache passes.
pub(crate) fn uses_indirect(program: &str) -> bool {
    tokens(program)
        .into_iter()
        .any(|t| INDIRECT_INPUTS.contains(&t))
}
/// Whether `program` declares `struct Parameters`, which composition binds as `params`.
pub(crate) fn declares_parameters(program: &str) -> bool {
    declares(program, "struct", "Parameters")
}

/// A diagnostic naming the program line when `offset` lies inside it.
fn diagnostic(
    label: &str,
    composed: &Composed,
    location: Option<naga::SourceLocation>,
    message: &str,
) -> Error {
    let first = composed.source[..composed.program.start]
        .matches('\n')
        .count() as u32
        + 1;
    Error::Shader(match location {
        Some(l) if composed.program.contains(&(l.offset as usize)) => format!(
            "program `{label}` line {}:{}: {message}",
            l.line_number - first + 1,
            l.line_position
        ),
        Some(l) => format!(
            "program `{label}`, engine composition line {}: {message}",
            l.line_number
        ),
        None => format!("program `{label}`: {message}"),
    })
}
fn in_program(composed: &Composed, span: naga::Span) -> bool {
    span.to_range()
        .is_some_and(|r| composed.program.contains(&r.start))
}

/// What the engine reads from a valid program: its `Parameters` layout, if declared.
#[derive(Debug)]
pub(crate) struct Analysis {
    pub parameters: Option<ParameterLayout>,
}

/// Checks a program's raster composition with naga before any device compile: both
/// functions, syntax, types and validation with program-line diagnostics, no program
/// bindings, and the reflected `Parameters` block.
pub(crate) fn analyse(label: &str, program: &str, composed: &Composed) -> Result<Analysis, Error> {
    for name in ["surface", "shade"] {
        if !declares(program, "fn", name) {
            return Err(Error::Shader(format!(
                "program `{label}` must define fn {name}; see shaders/programs/standard.wgsl"
            )));
        }
    }
    let source = &composed.source;
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| diagnostic(label, composed, e.location(source), e.message()))?;
    for (handle, global) in module.global_variables.iter() {
        let span = module.global_variables.get_span(handle);
        if global.binding.is_some() && in_program(composed, span) {
            return Err(diagnostic(
                label,
                composed,
                Some(span.location(source)),
                "programs cannot declare bindings; declare struct Parameters and use the engine services",
            ));
        }
    }
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| {
        let mut message = e.as_inner().to_string();
        let mut cause = std::error::Error::source(e.as_inner());
        while let Some(next) = cause {
            message = format!("{message}: {next}");
            cause = next.source();
        }
        let location = e
            .spans()
            .find(|(s, _)| in_program(composed, *s))
            .map(|(s, _)| s.location(source))
            .or_else(|| match e.as_inner() {
                naga::valid::ValidationError::GlobalVariable { handle, .. } => {
                    let span = module.types.get_span(module.global_variables[*handle].ty);
                    in_program(composed, span).then(|| span.location(source))
                }
                _ => None,
            })
            .or_else(|| e.location(source));
        diagnostic(label, composed, location, &message)
    })?;
    // The optional bounce hooks must match cell shading's calls (M7 G2).
    if reflects(program) {
        let checked = format!("{source}\n{}", reflection_wrappers(program));
        let valid = naga::front::wgsl::parse_str(&checked)
            .map_err(|e| e.message().to_string())
            .and_then(|m| {
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&m)
                .map(|_| ())
                .map_err(|e| e.as_inner().to_string())
            });
        if let Err(message) = valid {
            return Err(Error::Shader(format!(
                "program `{label}`: bounce light calls fn scatter(s: Surface, input: SurfaceInput, wi: vec3<f32>, wo: vec3<f32>) -> vec3<f32> and fn emission(s: Surface, input: SurfaceInput) -> vec3<f32>: {message}"
            )));
        }
    }
    let parameters = module
        .types
        .iter()
        .find(|(h, ty)| {
            ty.name.as_deref() == Some("Parameters")
                && in_program(composed, module.types.get_span(*h))
        })
        .map(|(handle, ty)| {
            reflect(&module, handle, ty).map_err(|message| {
                diagnostic(
                    label,
                    composed,
                    Some(module.types.get_span(handle).location(source)),
                    message,
                )
            })
        })
        .transpose()?;
    Ok(Analysis { parameters })
}

fn reflect(
    module: &naga::Module,
    handle: naga::Handle<naga::Type>,
    ty: &naga::Type,
) -> Result<ParameterLayout, &'static str> {
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        return Err("Parameters must be a struct");
    };
    if *span as usize > PARAMETER_BYTES {
        return Err("Parameters may use at most 256 bytes");
    }
    let fields = members
        .iter()
        .map(|m| {
            let kind = reflect_kind(module, m.ty)?;
            Ok(Field {
                name: m.name.clone().unwrap_or_default(),
                kind,
                offset: m.offset,
            })
        })
        .collect::<Result<_, _>>()?;
    let mut defaults = [0; PARAMETER_BYTES];
    if let Some((_, constant)) = module
        .constants
        .iter()
        .find(|(_, c)| c.name.as_deref() == Some("DEFAULT_PARAMETERS"))
    {
        if constant.ty != handle {
            return Err("DEFAULT_PARAMETERS must be a const of type Parameters");
        }
        write_constant(module, constant.init, &mut defaults)?;
    }
    Ok(ParameterLayout { fields, defaults })
}

fn reflect_kind(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Result<Kind, &'static str> {
    Ok(match module.types[ty].inner {
        naga::TypeInner::Scalar(s) if s == naga::Scalar::F32 => Kind::F32,
        naga::TypeInner::Scalar(s) if s == naga::Scalar::U32 => Kind::U32,
        naga::TypeInner::Scalar(s) if s == naga::Scalar::I32 => Kind::I32,
        naga::TypeInner::Vector { size, scalar } if scalar == naga::Scalar::F32 => match size {
            naga::VectorSize::Bi => Kind::Vec2,
            naga::VectorSize::Tri => Kind::Vec3,
            naga::VectorSize::Quad => Kind::Vec4,
        },
        naga::TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(len),
            stride,
        } => {
            let element = reflect_kind(module, base)?;
            if matches!(element, Kind::Array { .. }) {
                return Err("Parameters supports one-dimensional fixed arrays only");
            }
            Kind::Array {
                element: Box::new(element),
                len: len.get(),
                stride,
            }
        }
        _ => {
            return Err(
                "Parameters members must be f32, vec2/3/4<f32>, u32, i32 or fixed arrays of these",
            );
        }
    })
}

/// WGSL's frontend evaluates const expressions; pack that IR, never parse WGSL values
/// or reimplement its expression evaluator. Struct offsets and array strides come
/// from the same naga types that validate both GPU bindings.
fn write_constant(
    module: &naga::Module,
    init: naga::Handle<naga::Expression>,
    out: &mut [u8],
) -> Result<usize, &'static str> {
    use naga::{Expression as E, Literal as L, TypeInner as T};
    Ok(match &module.global_expressions[init] {
        E::Constant(c) => write_constant(module, module.constants[*c].init, out)?,
        E::ZeroValue(ty) => {
            let size = module.types[*ty].inner.size(module.to_ctx()) as usize;
            out[..size].fill(0);
            size
        }
        E::Literal(l) => {
            out[..4].copy_from_slice(&match l {
                L::F32(v) => v.to_ne_bytes(),
                L::U32(v) => v.to_ne_bytes(),
                L::I32(v) => v.to_ne_bytes(),
                _ => return Err("unsupported DEFAULT_PARAMETERS scalar"),
            });
            4
        }
        E::Splat { size, value } => {
            for dst in out.chunks_exact_mut(4).take(*size as usize) {
                write_constant(module, *value, dst)?;
            }
            *size as usize * 4
        }
        E::Compose { ty, components } => {
            match &module.types[*ty].inner {
                T::Struct { members, .. } => {
                    for (member, value) in members.iter().zip(components) {
                        write_constant(module, *value, &mut out[member.offset as usize..])?;
                    }
                }
                T::Array { stride, .. } => {
                    for (value, dst) in components.iter().zip(out.chunks_mut(*stride as usize)) {
                        write_constant(module, *value, dst)?;
                    }
                }
                T::Vector { .. } => {
                    let mut offset = 0;
                    for value in components {
                        offset += write_constant(module, *value, &mut out[offset..])?;
                    }
                }
                _ => return Err("unsupported DEFAULT_PARAMETERS composition"),
            }
            module.types[*ty].inner.size(module.to_ctx()) as usize
        }
        _ => return Err("DEFAULT_PARAMETERS must be a WGSL const expression"),
    })
}

/// Names the program location of a traced-stage rejection: implicit-derivative sampling
/// and dpdx/dpdy, which compute stages lack. None when the program has neither.
pub(crate) fn traced_stage_error(label: &str, composed: &Composed) -> Option<Error> {
    let module = naga::front::wgsl::parse_str(&composed.source).ok()?;
    module.functions.iter().find_map(|(_, f)| {
        f.expressions.iter().find_map(|(h, e)| {
            let implicit = matches!(
                e,
                naga::Expression::ImageSample {
                    level: naga::SampleLevel::Auto | naga::SampleLevel::Bias(_),
                    ..
                } | naga::Expression::Derivative { .. }
            );
            let span = f.expressions.get_span(h);
            (implicit && in_program(composed, span)).then(|| {
                diagnostic(
                    label,
                    composed,
                    Some(span.location(&composed.source)),
                    "traced shading has no implicit derivatives: use the sample_* helpers or sample_*_grad with input.uv_dx/uv_dy, not textureSample or dpdx/dpdy",
                )
            })
        })
    })
}

/// The standard program returning `value`, a vec3 expression, in place of its lit sum:
/// how tests probe one term (visibility, base colour) through the full program path.
#[cfg(test)]
pub(crate) fn standard_returning(value: &str) -> String {
    assert_eq!(
        STANDARD_PROGRAM
            .matches(
                "    return exposed;
"
            )
            .count(),
        1
    );
    STANDARD_PROGRAM.replace(
        "    return exposed;
",
        &format!(
            "    return {value};
"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const PRELUDE: &str = include_str!("../shaders/scene.wgsl");
    const TOON: &str = "struct Parameters { shadow_tint: vec3<f32>, bands: f32, rim_color: vec3<f32>, rim_power: f32, fill: f32, flat_shaded: u32 };\nstruct Surface { albedo: vec3<f32>, n: vec3<f32> };\nfn surface(input: SurfaceInput) -> Surface {\n    var n=input.geometric;\n    if params.flat_shaded==0u { n=normalize(input.normal); }\n    return Surface(input.material.base_color.rgb*sample_base_color(input).rgb,n);\n}\nfn shade(s: Surface, input: SurfaceInput) -> vec3<f32> {\n    var lit=ambient(s.n)*params.fill;\n    for(var i=0u;i<light_count();i+=1u) {\n        let light=light_sample(i,input);\n        let incident=max(dot(s.n,light.direction),0.0)*light_visibility(input,light);\n        let band=floor(incident*params.bands)/params.bands;\n        lit+=light.intensity*light.attenuation*band;\n    }\n    return expose(s.albedo*lit/PI);\n}\n";

    fn raster(program: &str) -> Composed {
        compose(
            PRELUDE,
            program,
            Stage::Raster,
            declares_parameters(program),
        )
    }
    fn rejection(label: &str, program: &str) -> String {
        match analyse(label, program, &raster(program)) {
            Err(Error::Shader(message)) => message,
            other => panic!("expected a shader rejection, got {other:?}"),
        }
    }

    #[test]
    fn default_traced_shading_has_no_residency_code() {
        let head = format!("const PRIMARY_BATCHES:u32=64u;\n{PRELUDE}");
        let plain = compose(&head, STANDARD_PROGRAM, Stage::Traced, false).source;
        let streamed = compose(&head, STANDARD_PROGRAM, Stage::TracedStreamed, false).source;
        assert!(plain.contains(TRACED_API));
        for name in ["primary_residency", "primary_level_probe", "rl_record"] {
            assert!(!plain.contains(name) && streamed.contains(name), "{name}");
        }
        // Exactly the three `_grad` helpers are replaced, once each.
        for helper in ["base_color", "metallic_roughness", "normal_map"] {
            let definition = format!("fn sample_{helper}_grad(");
            assert_eq!(plain.matches(&definition).count(), 1);
            assert_eq!(streamed.matches(&definition).count(), 1);
        }
        assert!(
            streamed.contains("fn environment_dfg(") && streamed.contains("fn sample_base_color(")
        );
    }

    #[test]
    fn standard_program_composes_without_parameters() {
        let composed = raster(STANDARD_PROGRAM);
        assert!(!declares_parameters(STANDARD_PROGRAM));
        assert_eq!(&composed.source[composed.program.clone()], STANDARD_PROGRAM);
        assert!(
            analyse("standard", STANDARD_PROGRAM, &composed)
                .unwrap()
                .parameters
                .is_none()
        );
    }

    #[test]
    fn specular_filter_helpers_and_custom_geometric_inputs_compose() {
        analyse("specular AA", STANDARD_PROGRAM, &raster(STANDARD_PROGRAM)).unwrap();
        // A program can consume the geometric inputs without adopting GGX.
        let custom = "struct Surface { variance:f32 }; fn surface(input:SurfaceInput)->Surface { return Surface(geometric_normal_variance(input)); } fn shade(s:Surface,input:SurfaceInput)->vec3<f32> { return vec3<f32>(s.variance); }";
        analyse("custom variance", custom, &raster(custom)).unwrap();
    }

    #[test]
    fn parameters_are_reflected_checked_and_packed() {
        let layout = analyse("toon", TOON, &raster(TOON))
            .unwrap()
            .parameters
            .unwrap();
        let offsets: Vec<_> = layout
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.offset))
            .collect();
        assert_eq!(
            offsets,
            [
                ("shadow_tint", 0),
                ("bands", 12),
                ("rim_color", 16),
                ("rim_power", 28),
                ("fill", 32),
                ("flat_shaded", 36)
            ]
        );
        let l = Some(&layout);
        assert!(ParameterLayout::check(l, [("bands", Value::F32(3.0))]).is_ok());
        assert!(ParameterLayout::check(l, [("bands", Value::U32(3))]).is_err());
        assert!(ParameterLayout::check(l, [("missing", Value::F32(1.0))]).is_err());
        assert!(ParameterLayout::check(None, [("bands", Value::F32(1.0))]).is_err());
        let block = ParameterLayout::pack(
            l,
            &[
                ("bands".into(), Value::F32(3.0)),
                ("flat_shaded".into(), Value::U32(1)),
            ],
        );
        assert_eq!(block[12..16], 3f32.to_ne_bytes());
        assert_eq!(block[36..40], 1u32.to_ne_bytes());
        assert!(
            block[..12]
                .iter()
                .chain(&block[16..36])
                .chain(&block[40..])
                .all(|&b| b == 0)
        );
        let retyped = TOON.replace("rim_power: f32", "rim_power: u32");
        let next = analyse("toon", &retyped, &raster(&retyped))
            .unwrap()
            .parameters;
        assert_eq!(ParameterLayout::changed(l, next.as_ref()), ["rim_power"]);
        assert!(ParameterLayout::changed(l, l).is_empty());
    }

    #[test]
    fn program_defaults_and_fixed_arrays_use_naga_layout() {
        let source = format!(
            "{}\n{STANDARD_PROGRAM}",
            r#"
struct Parameters { gain:f32, thresholds:array<vec3<f32>,2>, count:u32, signed:i32, tail:vec2<f32> };
const BASE:vec3<f32> = vec3<f32>(0.125,0.25,0.5);
const DEFAULT_PARAMETERS:Parameters = Parameters(
    2.0*0.25, array<vec3<f32>,2>(BASE,vec3<f32>(0.75)), 3u, -2, vec2<f32>(1.0,2.0));
"#
        );
        let layout = analyse("arrays", &source, &raster(&source))
            .unwrap()
            .parameters
            .unwrap();
        assert_eq!(
            layout.fields.iter().map(|f| f.offset).collect::<Vec<_>>(),
            [0, 16, 48, 52, 56]
        );
        let block = ParameterLayout::pack(Some(&layout), &[]);
        let word = |offset| f32::from_ne_bytes(block[offset..offset + 4].try_into().unwrap());
        assert_eq!(
            [word(0), word(16), word(20), word(24), word(32)],
            [0.5, 0.125, 0.25, 0.5, 0.75]
        );
        assert_eq!(&block[48..52], &3u32.to_ne_bytes());
        assert_eq!(&block[52..56], &(-2i32).to_ne_bytes());
        assert_eq!([word(56), word(60)], [1.0, 2.0]);
        assert!(block[28..32].iter().chain(&block[44..48]).all(|&v| v == 0));
        let array = Value::Array(vec![Value::Vec3([1., 2., 3.]), Value::Vec3([4., 5., 6.])]);
        ParameterLayout::check(Some(&layout), [("thresholds", array.clone())]).unwrap();
        for invalid in [
            Value::Array(vec![]),
            Value::Array(vec![Value::Vec3([1.; 3])]),
            Value::Array(vec![Value::Vec4([1.; 4]); 2]),
            Value::Vec3([1.; 3]),
        ] {
            assert!(ParameterLayout::check(Some(&layout), [("thresholds", invalid)]).is_err());
        }
        let overridden = ParameterLayout::pack(
            Some(&layout),
            &[
                ("thresholds".into(), array),
                ("gain".into(), Value::F32(0.)),
            ],
        );
        assert_eq!(&overridden[..4], &[0; 4]);
        assert_eq!(&overridden[32..36], &4f32.to_ne_bytes());
        assert_eq!(&overridden[48..64], &block[48..64]);
        let next_source = source.replace("2.0*0.25", "0.75");
        let next = analyse("arrays", &next_source, &raster(&next_source))
            .unwrap()
            .parameters
            .unwrap();
        assert_ne!(layout, next, "default edits trigger material repacking");
        assert!(ParameterLayout::changed(Some(&layout), Some(&next)).is_empty());
        assert!(
            rejection(
                "arrays",
                &format!(
                    "struct Parameters {{ oversized:array<vec4<f32>,17> }};\n{STANDARD_PROGRAM}"
                )
            )
            .contains("Parameters")
        );
        let scalar_array = format!("struct Parameters {{ a:array<f32,2> }};\n{STANDARD_PROGRAM}");
        let error = rejection("arrays", &scalar_array);
        assert!(error.contains("program `arrays` line 1:"), "{error}");
        let wrong = source.replace("const DEFAULT_PARAMETERS:Parameters = Parameters(\n    2.0*0.25, array<vec3<f32>,2>(BASE,vec3<f32>(0.75)), 3u, -2, vec2<f32>(1.0,2.0));", "const DEFAULT_PARAMETERS:f32 = 1.0;");
        assert!(rejection("arrays", &wrong).contains("DEFAULT_PARAMETERS"));
    }

    #[test]
    fn toon_defaults_equal_the_original_application_values() {
        let source = include_str!("../shaders/programs/toon.wgsl");
        let layout = analyse("toon", source, &raster(source))
            .unwrap()
            .parameters
            .unwrap();
        let explicit = [
            ("shadow_tint".into(), Value::Vec3([0.12, 0.08, 0.24])),
            ("bands".into(), Value::F32(4.)),
            ("rim_color".into(), Value::Vec3([0.12, 0.2, 0.3])),
            ("rim_power".into(), Value::F32(4.)),
            ("thresholds".into(), Value::Vec2([0.12, 0.9])),
            ("fill".into(), Value::F32(0.25)),
            ("flat_shaded".into(), Value::U32(0)),
        ];
        assert_eq!(
            ParameterLayout::pack(Some(&layout), &[]),
            ParameterLayout::pack(Some(&layout), &explicit)
        );
    }

    #[test]
    fn rejections_name_program_lines() {
        let line = |text: &str| {
            STANDARD_PROGRAM
                .lines()
                .position(|l| l.contains(text))
                .unwrap()
                + 1
        };
        let syntax = STANDARD_PROGRAM.replace(
            "let mr=sample_metallic_roughness(input);",
            "let mr=sample_metallic_roughness(input;",
        );
        let message = rejection("standard", &syntax);
        assert!(
            message.starts_with(&format!("program `standard` line {}:", line("let mr="))),
            "{message}"
        );
        let binding =
            format!("@group(2) @binding(9) var<uniform> mine:vec4<f32>;\n{STANDARD_PROGRAM}");
        let message = rejection("standard", &binding);
        assert!(
            message.starts_with("program `standard` line 1:") && message.contains("bindings"),
            "{message}"
        );
        let redefinition = format!("{STANDARD_PROGRAM}fn light_count() -> u32 {{ return 0u; }}\n");
        let message = rejection("standard", &redefinition);
        assert!(message.contains("light_count"), "{message}");
        let missing = STANDARD_PROGRAM.replace("fn shade(", "fn shaded(");
        assert!(rejection("standard", &missing).contains("must define fn shade"));
        // A commented-out definition does not count.
        let commented = STANDARD_PROGRAM.replace("fn surface(", "// fn surface(\nfn surfaced(");
        assert!(rejection("standard", &commented).contains("must define fn surface"));
        let oversized =
            format!("struct Parameters {{ a: array<vec4<f32>, 17> }};\n{STANDARD_PROGRAM}");
        assert!(rejection("standard", &oversized).contains("Parameters"));
    }
}
