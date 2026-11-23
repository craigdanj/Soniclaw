//! Declare typed controls once; DSP handles, host formatting and snapshots share the same definition.
use serde::{Deserialize, Serialize};
#[doc(hidden)]
pub use soniclaw_core as __core;
use soniclaw_core::{stable_parameter_id, Error, Parameter, ParameterCodec, ParameterInfo};
use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    sync::Arc,
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Curve {
    Linear,
    Logarithmic,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Kind {
    Float {
        min: f32,
        max: f32,
        default: f32,
        curve: Curve,
    },
    Integer {
        min: i32,
        max: i32,
        default: i32,
    },
    Boolean {
        default: bool,
    },
    Choice {
        labels: Vec<String>,
        default: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub group: String,
    pub kind: Kind,
}
impl Definition {
    pub fn new(id: &str, name: &str, kind: Kind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            unit: String::new(),
            group: String::new(),
            kind,
        }
    }
    pub fn unit(mut self, unit: &str) -> Self {
        self.unit = unit.into();
        self
    }
    pub fn group(mut self, group: &str) -> Self {
        self.group = group.into();
        self
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.id.is_empty()
            || self.id.len() > 128
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || self.name.is_empty()
            || self.name.len() > 128
            || self.unit.len() > 24
            || self.group.len() > 128
            || [&self.name, &self.unit, &self.group]
                .iter()
                .any(|s| s.contains('\0'))
            || stable_parameter_id(&self.id) == u32::MAX
        {
            return Err(Error::InvalidParameter);
        }
        let valid = match &self.kind {
            Kind::Float {
                min,
                max,
                default,
                curve,
            } => {
                [min, max, default].iter().all(|v| v.is_finite())
                    && min < max
                    && (*min..=*max).contains(default)
                    && (*curve != Curve::Logarithmic || *min > 0.)
                    && (*max - *min).is_finite()
            }
            Kind::Integer { min, max, default } => {
                min < max
                    && *min >= -16_777_216
                    && *max <= 16_777_216
                    && (*max as i64 - *min as i64) <= 65535
                    && (*min..=*max).contains(default)
            }
            Kind::Boolean { .. } => true,
            Kind::Choice { labels, default } => {
                (2..=256).contains(&labels.len())
                    && *default < labels.len()
                    && labels.iter().all(|l| {
                        !l.is_empty() && l.len() <= 128 && l.trim() == l && !l.contains('\0')
                    })
                    && labels
                        .iter()
                        .map(|l| l.to_lowercase())
                        .collect::<BTreeSet<_>>()
                        .len()
                        == labels.len()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidParameter)
        }
    }
    fn raw_range(&self) -> (f32, f32, f32) {
        match &self.kind {
            Kind::Float {
                min,
                max,
                default,
                curve,
            } => (0., 1., normalize(*default, *min, *max, *curve)),
            Kind::Integer { min, max, default } => (*min as f32, *max as f32, *default as f32),
            Kind::Boolean { default } => (0., 1., u8::from(*default) as f32),
            Kind::Choice { labels, default } => (0., (labels.len() - 1) as f32, *default as f32),
        }
    }
    pub fn physical(&self, raw: f32) -> f32 {
        match self.kind {
            Kind::Float {
                min, max, curve, ..
            } => denormalize(raw, min, max, curve),
            _ => raw,
        }
    }
    pub fn raw_from_physical(&self, value: f32) -> Result<f32, Error> {
        if !value.is_finite() {
            return Err(Error::InvalidParameter);
        }
        match self.kind {
            Kind::Float {
                min, max, curve, ..
            } if (min..=max).contains(&value) => Ok(normalize(value, min, max, curve)),
            Kind::Float { .. } => Err(Error::InvalidParameter),
            _ => {
                let (min, max, _) = self.raw_range();
                if (min..=max).contains(&value) {
                    Ok(self.quantize(value))
                } else {
                    Err(Error::InvalidParameter)
                }
            }
        }
    }
}
fn normalize(v: f32, min: f32, max: f32, curve: Curve) -> f32 {
    match curve {
        Curve::Linear => (v - min) / (max - min),
        Curve::Logarithmic => (v.ln() - min.ln()) / (max.ln() - min.ln()),
    }
    .clamp(0., 1.)
}
fn denormalize(n: f32, min: f32, max: f32, curve: Curve) -> f32 {
    let n = n.clamp(0., 1.);
    match curve {
        Curve::Linear => min + n * (max - min),
        Curve::Logarithmic => (min.ln() + n * (max.ln() - min.ln())).exp(),
    }
    .clamp(min, max)
}
impl ParameterCodec for Definition {
    fn quantize(&self, v: f32) -> f32 {
        match self.kind {
            Kind::Float { .. } => v,
            _ => v.round(),
        }
    }
    fn format(&self, raw: f32) -> String {
        match &self.kind {
            Kind::Boolean { .. } => if raw >= 0.5 { "On" } else { "Off" }.into(),
            Kind::Choice { labels, .. } => labels
                .get(raw.round().clamp(0., (labels.len() - 1) as f32) as usize)
                .cloned()
                .unwrap_or_default(),
            Kind::Integer { .. } => format!(
                "{}{}{}",
                raw.round() as i32,
                if self.unit.is_empty() { "" } else { " " },
                self.unit
            ),
            Kind::Float { .. } => format!(
                "{:.3}{}{}",
                self.physical(raw),
                if self.unit.is_empty() { "" } else { " " },
                self.unit
            ),
        }
    }
    fn parse(&self, text: &str) -> Option<f32> {
        let text = text.trim();
        match &self.kind {
            Kind::Boolean { .. } => match text.to_ascii_lowercase().as_str() {
                "on" | "true" | "1" => Some(1.),
                "off" | "false" | "0" => Some(0.),
                _ => None,
            },
            Kind::Choice { labels, .. } => labels
                .iter()
                .position(|l| l.eq_ignore_ascii_case(text))
                .map(|i| i as f32),
            _ => {
                let text = if self.unit.is_empty() {
                    text
                } else {
                    text.strip_suffix(&self.unit).unwrap_or(text).trim()
                };
                self.raw_from_physical(text.parse().ok()?).ok()
            }
        }
    }
    fn step_count(&self) -> u32 {
        match &self.kind {
            Kind::Float { .. } => 0,
            Kind::Integer { min, max, .. } => (*max as i64 - *min as i64) as u32,
            Kind::Boolean { .. } => 1,
            Kind::Choice { labels, .. } => (labels.len() - 1) as u32,
        }
    }
    fn is_enum(&self) -> bool {
        matches!(self.kind, Kind::Choice { .. } | Kind::Boolean { .. })
    }
    fn group(&self) -> &str {
        &self.group
    }
}
pub trait ValueType: Send + Sync {
    type Value;
    fn accepts(kind: &Kind) -> bool;
    fn decode(d: &Definition, raw: f32) -> Self::Value;
    fn encode(d: &Definition, value: Self::Value) -> Result<f32, Error>;
}
pub enum Float {}
pub enum Integer {}
pub enum Boolean {}
pub enum Choice {}
impl ValueType for Float {
    type Value = f32;
    fn accepts(k: &Kind) -> bool {
        matches!(k, Kind::Float { .. })
    }
    fn decode(d: &Definition, v: f32) -> f32 {
        d.physical(v)
    }
    fn encode(d: &Definition, v: f32) -> Result<f32, Error> {
        d.raw_from_physical(v)
    }
}
impl ValueType for Integer {
    type Value = i32;
    fn accepts(k: &Kind) -> bool {
        matches!(k, Kind::Integer { .. })
    }
    fn decode(_: &Definition, v: f32) -> i32 {
        v.round() as i32
    }
    fn encode(d: &Definition, v: i32) -> Result<f32, Error> {
        let Kind::Integer { min, max, .. } = d.kind else {
            return Err(Error::InvalidParameter);
        };
        if !(min..=max).contains(&v) {
            return Err(Error::InvalidParameter);
        }
        Ok(v as f32)
    }
}
impl ValueType for Boolean {
    type Value = bool;
    fn accepts(k: &Kind) -> bool {
        matches!(k, Kind::Boolean { .. })
    }
    fn decode(_: &Definition, v: f32) -> bool {
        v >= 0.5
    }
    fn encode(_: &Definition, v: bool) -> Result<f32, Error> {
        Ok(u8::from(v) as f32)
    }
}
impl ValueType for Choice {
    type Value = usize;
    fn accepts(k: &Kind) -> bool {
        matches!(k, Kind::Choice { .. })
    }
    fn decode(_: &Definition, v: f32) -> usize {
        v.round() as usize
    }
    fn encode(d: &Definition, v: usize) -> Result<f32, Error> {
        let Kind::Choice { ref labels, .. } = d.kind else {
            return Err(Error::InvalidParameter);
        };
        if v >= labels.len() {
            return Err(Error::InvalidParameter);
        }
        Ok(v as f32)
    }
}
pub struct Typed<T: ValueType> {
    definition: Arc<Definition>,
    raw: Arc<Parameter>,
    _type: PhantomData<T>,
}
impl<T: ValueType> Clone for Typed<T> {
    fn clone(&self) -> Self {
        Self {
            definition: self.definition.clone(),
            raw: self.raw.clone(),
            _type: PhantomData,
        }
    }
}
impl<T: ValueType> Typed<T> {
    pub fn new(definition: Definition) -> Result<Self, Error> {
        definition.validate()?;
        if !T::accepts(&definition.kind) {
            return Err(Error::InvalidParameter);
        }
        let definition = Arc::new(definition);
        let (min, max, default) = definition.raw_range();
        let raw = Arc::new(Parameter::with_codec(
            ParameterInfo {
                id: &definition.id,
                name: &definition.name,
                unit: &definition.unit,
                min,
                max,
                default,
            },
            definition.clone(),
        )?);
        Ok(Self {
            definition,
            raw,
            _type: PhantomData,
        })
    }
    pub fn get(&self) -> T::Value {
        T::decode(&self.definition, self.raw.get())
    }
    pub fn modulated(&self) -> T::Value {
        T::decode(
            &self.definition,
            self.definition.quantize(self.raw.modulated()),
        )
    }
    pub fn set(&self, value: T::Value) -> Result<(), Error> {
        self.raw.set(T::encode(&self.definition, value)?)
    }
    pub fn raw(&self) -> &Arc<Parameter> {
        &self.raw
    }
    pub fn definition(&self) -> &Definition {
        &self.definition
    }
    pub fn normalized(&self) -> f32 {
        self.raw.normalized()
    }
    pub fn set_normalized(&self, value: f32) -> Result<(), Error> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(Error::InvalidParameter);
        }
        self.raw.set_normalized(value)
    }
}
pub type FloatParam = Typed<Float>;
pub type IntParam = Typed<Integer>;
pub type BoolParam = Typed<Boolean>;
pub type ChoiceParam = Typed<Choice>;
/// A fieldless enum with stable ordered raw indices. Revise/migrate normalized snapshots when choices change.
pub trait EnumChoice: Copy + Send + Sync + 'static {
    fn variants() -> &'static [Self];
    fn index(self) -> usize;
    fn label(self) -> &'static str;
}
pub struct EnumParam<T: EnumChoice> {
    inner: ChoiceParam,
    _type: PhantomData<T>,
}
impl<T: EnumChoice> Clone for EnumParam<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _type: PhantomData,
        }
    }
}
impl<T: EnumChoice> EnumParam<T> {
    pub fn new(id: &str, name: &str, default: T) -> Result<Self, Error> {
        let variants = T::variants();
        if !(2..=256).contains(&variants.len())
            || default.index() >= variants.len()
            || variants.iter().enumerate().any(|(i, v)| v.index() != i)
        {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            inner: ChoiceParam::new(Definition::new(
                id,
                name,
                Kind::Choice {
                    labels: variants.iter().map(|v| v.label().to_owned()).collect(),
                    default: default.index(),
                },
            ))?,
            _type: PhantomData,
        })
    }
    pub fn get(&self) -> T {
        T::variants()[self.inner.get()]
    }
    pub fn modulated(&self) -> T {
        T::variants()[self.inner.modulated()]
    }
    pub fn set(&self, value: T) -> Result<(), Error> {
        self.inner.set(value.index())
    }
    pub fn raw(&self) -> &Arc<Parameter> {
        self.inner.raw()
    }
    pub fn definition(&self) -> &Definition {
        self.inner.definition()
    }
    pub fn normalized(&self) -> f32 {
        self.inner.normalized()
    }
    pub fn set_normalized(&self, value: f32) -> Result<(), Error> {
        self.inner.set_normalized(value)
    }
}
#[macro_export]
macro_rules! enum_choices{($visibility:vis enum $name:ident {$($variant:ident => $label:literal),+$(,)?})=>{
 #[derive(Clone,Copy,Debug,PartialEq,Eq)] #[repr(usize)] $visibility enum $name{$($variant),+}
 impl $crate::EnumChoice for $name{fn variants()->&'static[Self]{&[$(Self::$variant),+]}fn index(self)->usize{self as usize}fn label(self)->&'static str{match self{$(Self::$variant=>$label),+}}}
};}
pub fn validate_set(parameters: &[Arc<Parameter>]) -> Result<(), Error> {
    if parameters.len() > 4096 {
        return Err(Error::InvalidParameter);
    }
    let mut ids = BTreeSet::new();
    for p in parameters {
        let id = stable_parameter_id(p.info().id);
        if p.info().id.is_empty() || p.info().id.len() > 128 || id == u32::MAX || !ids.insert(id) {
            return Err(Error::InvalidParameter);
        }
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub product: String,
    pub declaration_revision: u32,
    pub values: BTreeMap<String, f32>,
}
impl Snapshot {
    pub fn capture(
        product: &str,
        revision: u32,
        parameters: &[Arc<Parameter>],
    ) -> Result<Self, Error> {
        validate_set(parameters)?;
        if product.is_empty() || product.len() > 256 || revision == 0 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            schema: 1,
            product: product.into(),
            declaration_revision: revision,
            values: parameters
                .iter()
                .map(|p| (p.info().id.to_owned(), p.normalized()))
                .collect(),
        })
    }
    pub fn restore(
        &self,
        product: &str,
        revision: u32,
        parameters: &[Arc<Parameter>],
    ) -> Result<(), Error> {
        validate_set(parameters)?;
        if self.schema != 1
            || self.product.is_empty()
            || self.product.len() > 256
            || self.declaration_revision == 0
            || self.product != product
            || self.declaration_revision != revision
            || self.values.len() > 4096
            || self.values.iter().any(|(id, v)| {
                !v.is_finite()
                    || !(0.0..=1.0).contains(v)
                    || !parameters.iter().any(|p| p.info().id == id)
            })
        {
            return Err(Error::InvalidParameter);
        }
        for p in parameters {
            if let Some(v) = self.values.get(p.info().id) {
                p.set_normalized(*v)?;
            }
        }
        Ok(())
    }
    pub fn from_json(text: &str) -> Result<Self, Error> {
        if text.len() > 1024 * 1024 {
            return Err(Error::InvalidParameter);
        }
        let snapshot: Self = serde_json::from_str(text).map_err(|_| Error::InvalidParameter)?;
        if snapshot.schema != 1
            || snapshot.product.is_empty()
            || snapshot.product.len() > 256
            || snapshot.declaration_revision == 0
            || snapshot.values.len() > 4096
            || snapshot.values.iter().any(|(id, v)| {
                id.is_empty() || id.len() > 128 || !v.is_finite() || !(0.0..=1.0).contains(v)
            })
        {
            return Err(Error::InvalidParameter);
        }
        Ok(snapshot)
    }
}
/// Define typed fields and generate construction, host/raw list and declaration export from the same handles.
#[macro_export]
macro_rules! parameter_set{($visibility:vis struct $name:ident{$($field:ident:$ty:ty=$init:expr),*$(,)?})=>{
 $visibility struct $name{$(pub $field:$ty),*}
 impl $name{
  pub fn new()->::std::result::Result<Self,$crate::__core::Error>{let result=Self{$($field:$init?),*};$crate::validate_set(&result.raw_parameters())?;Ok(result)}
  pub fn raw_parameters(&self)->::std::vec::Vec<::std::sync::Arc<$crate::__core::Parameter>>{::std::vec![$(self.$field.raw().clone()),*]}
  pub fn declarations(&self)->::std::vec::Vec<$crate::Definition>{::std::vec![$(self.$field.definition().clone()),*]}
 }
};}
#[cfg(test)]
mod tests {
    use super::*;
    parameter_set! {pub struct Controls{cutoff:FloatParam=FloatParam::new(Definition::new("filter.cutoff","Cutoff",Kind::Float{min:20.,max:20000.,default:200.,curve:Curve::Logarithmic}).unit("Hz").group("Filter")),voices:IntParam=IntParam::new(Definition::new("voices","Voices",Kind::Integer{min:1,max:64,default:8})),enabled:BoolParam=BoolParam::new(Definition::new("enabled","Enabled",Kind::Boolean{default:true})),mode:ChoiceParam=ChoiceParam::new(Definition::new("mode","Mode",Kind::Choice{labels:vec!["Low pass".into(),"High pass".into(),"Band pass".into()],default:0}))}}
    #[test]
    fn dsp_host_ui_share_mapping_types_and_text() {
        let p = Controls::new().unwrap();
        assert!((p.cutoff.normalized() - 1. / 3.).abs() < 1e-6);
        p.cutoff.set_normalized(0.5).unwrap();
        assert!((p.cutoff.get() - (20_f32 * 20000.).sqrt()).abs() < 0.001);
        assert!(p.cutoff.raw().format_value(0.5).ends_with("Hz"));
        let n = p.cutoff.raw().parse_value("1000 Hz").unwrap();
        p.cutoff.raw().set(n).unwrap();
        assert!((p.cutoff.get() - 1000.).abs() < 0.001);
        p.voices.set_normalized(0.5).unwrap();
        assert_eq!(p.voices.get(), 33);
        assert_eq!(p.voices.raw().step_count(), 63);
        p.mode.raw().set(1.7).unwrap();
        assert_eq!(p.mode.get(), 2);
        assert_eq!(p.mode.raw().format_value(2.), "Band pass");
        assert_eq!(p.mode.raw().parse_value("high pass"), Some(1.));
        assert!(p.mode.set(4).is_err());
        assert!(p.cutoff.set(f32::NAN).is_err());
        assert_eq!(p.declarations().len(), 4);
        assert!(p.enabled.get());
    }
    #[test]
    fn normalized_snapshot_roundtrip_rejects_wrong_revision_before_mutation() {
        let p = Controls::new().unwrap();
        p.cutoff.set(1200.).unwrap();
        let snapshot = Snapshot::capture("org.test", 1, &p.raw_parameters()).unwrap();
        let text = serde_json::to_string(&snapshot).unwrap();
        let copy = Snapshot::from_json(&text).unwrap();
        p.cutoff.set(20.).unwrap();
        assert!(copy.restore("org.test", 2, &p.raw_parameters()).is_err());
        assert!((p.cutoff.get() - 20.).abs() < 0.001);
        copy.restore("org.test", 1, &p.raw_parameters()).unwrap();
        assert!((p.cutoff.get() - 1200.).abs() < 0.001);
    }
    #[test]
    fn callback_reads_writes_and_quantization_allocate_nothing() {
        let p = Controls::new().unwrap();
        let measure = allocation_counter::measure(|| {
            for i in 0..4096 {
                p.cutoff.set_normalized(i as f32 / 4096.).unwrap();
                assert!(p.cutoff.get().is_finite());
                p.voices.set((i % 64) + 1).unwrap();
                p.mode.set((i % 3) as usize).unwrap();
                p.enabled.set(i % 2 == 0).unwrap();
            }
        });
        assert_eq!(measure.count_total, 0);
    }
    #[test]
    fn declarations_reject_collisions_and_invalid_ranges() {
        let a = FloatParam::new(Definition::new(
            "x",
            "X",
            Kind::Float {
                min: 0.,
                max: 1.,
                default: 0.5,
                curve: Curve::Logarithmic,
            },
        ));
        assert!(a.is_err());
        let p = Controls::new().unwrap();
        assert!(validate_set(&[p.cutoff.raw().clone(), p.cutoff.raw().clone()]).is_err());
        let mut def = p.mode.definition().clone();
        if let Kind::Choice { labels, .. } = &mut def.kind {
            labels[1] = labels[0].clone();
        }
        assert!(def.validate().is_err());
    }
    enum_choices! {pub enum FilterMode{Low=>"Low pass",High=>"High pass",Band=>"Band pass"}}
    #[test]
    fn real_rust_enums_keep_typed_value_and_host_labels() {
        let p = EnumParam::<FilterMode>::new("mode", "Mode", FilterMode::Low).unwrap();
        p.set(FilterMode::High).unwrap();
        assert_eq!(p.get(), FilterMode::High);
        assert_eq!(p.raw().format_value(p.raw().get()), "High pass");
        p.set_normalized(1.).unwrap();
        assert_eq!(p.get(), FilterMode::Band);
    }
    #[test]
    fn bounded_integer_snapshot_preserves_extreme_representable_steps() {
        let p = IntParam::new(Definition::new(
            "int",
            "Int",
            Kind::Integer {
                min: 16_711_681,
                max: 16_777_216,
                default: 16_711_681,
            },
        ))
        .unwrap();
        for value in [16_711_681, 16_711_682, 16_750_001, 16_777_215, 16_777_216] {
            p.set(value).unwrap();
            let n = p.normalized();
            p.set_normalized(n).unwrap();
            assert_eq!(p.get(), value);
        }
    }
}
