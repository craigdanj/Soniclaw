//! Common trusted product/control surface. Processing remains backend-specific.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum PluginFormat {
    Clap,
    Vst3,
}
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct Product {
    pub format: PluginFormat,
    pub path: PathBuf,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct NormalizedParameter {
    pub id: u32,
    pub name: String,
    pub value: f64,
    pub default: f64,
    pub read_only: bool,
}
/// # Safety
/// Discovery executes native code. Select trusted libraries. Run in a supervised child to contain scan crashes/hangs.
pub unsafe fn products(format: PluginFormat, path: &Path) -> Result<Vec<Product>, HostError> {
    match format {
        PluginFormat::Clap => {
            let library = unsafe { ClapLibrary::load(path) }?;
            Ok(library
                .plugins()
                .iter()
                .map(|p| Product {
                    format,
                    path: path.to_owned(),
                    id: p.id.clone(),
                    name: p.name.clone(),
                    vendor: p.vendor.clone(),
                    version: p.version.clone(),
                })
                .collect())
        }
        PluginFormat::Vst3 => {
            #[cfg(feature = "vst3")]
            {
                let report = unsafe { soniclaw_vst3::probe(path) }?;
                Ok(report
                    .classes
                    .into_iter()
                    .filter(|c| c.category == "Audio Module Class")
                    .map(|c| Product {
                        format,
                        path: path.to_owned(),
                        id: c.class_id,
                        name: c.name,
                        vendor: report.factory.vendor.clone(),
                        version: c.version,
                    })
                    .collect())
            }
            #[cfg(not(feature = "vst3"))]
            {
                Err("enable the vst3 host feature".into())
            }
        }
    }
}
/// Main/control-thread instances. Native processing/editor methods are available on each variant.
pub enum Instance {
    Clap(ClapInstance),
    #[cfg(feature = "vst3")]
    Vst3(soniclaw_vst3::Instance),
}
impl Instance {
    /// # Safety
    /// The selected product must point to trusted native code on the GUI/main thread.
    pub unsafe fn load(product: &Product, spec: ProcessSpec) -> Result<Self, HostError> {
        match product.format {
            PluginFormat::Clap => {
                let library = unsafe { ClapLibrary::load(&product.path) }?;
                Ok(Self::Clap(library.instantiate(&product.id)?))
            }
            PluginFormat::Vst3 => {
                #[cfg(feature = "vst3")]
                {
                    Ok(Self::Vst3(unsafe {
                        soniclaw_vst3::Instance::load(&product.path, Some(&product.id), spec)
                    }?))
                }
                #[cfg(not(feature = "vst3"))]
                {
                    let _ = spec;
                    Err("enable the vst3 host feature".into())
                }
            }
        }
    }
    pub fn parameters(&self) -> Result<Vec<NormalizedParameter>, HostError> {
        match self {
            Self::Clap(p) => Ok(p
                .parameters()
                .iter()
                .map(|p| NormalizedParameter {
                    id: p.id,
                    name: p.name.clone(),
                    value: normalize(p.value, p.min, p.max),
                    default: normalize(p.default, p.min, p.max),
                    read_only: p.flags & clap_sys::ext::params::CLAP_PARAM_IS_READONLY != 0,
                })
                .collect()),
            #[cfg(feature = "vst3")]
            Self::Vst3(p) => Ok(p
                .parameters()?
                .into_iter()
                .map(|p| NormalizedParameter {
                    id: p.id,
                    name: p.name,
                    value: p.value,
                    default: p.default,
                    read_only: p.is_read_only,
                })
                .collect()),
        }
    }
    pub fn set_normalized(&mut self, id: u32, value: f64) -> Result<(), HostError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("normalized value must be finite and 0..1".into());
        }
        match self {
            Self::Clap(p) => {
                let parameter = p
                    .parameters()
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or("unknown parameter")?;
                let value = parameter.min + value * (parameter.max - parameter.min);
                p.restore_parameters(&BTreeMap::from([(id, value)]))
            }
            #[cfg(feature = "vst3")]
            Self::Vst3(p) => p.set_parameter(id, value),
        }
    }
    pub fn latency_frames(&self) -> Result<usize, HostError> {
        match self {
            Self::Clap(p) => Ok(p.latency_frames()),
            #[cfg(feature = "vst3")]
            Self::Vst3(p) => p.latency_frames(),
        }
    }
}
fn normalize(v: f64, min: f64, max: f64) -> f64 {
    if max > min {
        ((v - min) / (max - min)).clamp(0., 1.)
    } else {
        0.
    }
}

/// Explicit native-code trust capability, bound to its constructing thread.
/// Safe reconstruction APIs require this token rather than executing paths merely parsed from JSON.
#[derive(Default)]
pub struct TrustedLoader {
    paths: std::collections::BTreeSet<PathBuf>,
    _thread: std::marker::PhantomData<Rc<()>>,
}
impl TrustedLoader {
    /// # Safety
    /// Every selected native file/bundle and its dependencies must remain trusted,
    /// ABI-conforming and unmodified for this capability's lifetime. Construct on the GUI/main thread.
    pub unsafe fn new(paths: &[PathBuf]) -> Result<Self, HostError> {
        if paths.len() > 256 {
            return Err("at most 256 trusted module paths".into());
        }
        let paths = paths
            .iter()
            .map(|p| p.canonicalize())
            .collect::<Result<_, _>>()?;
        Ok(Self {
            paths,
            _thread: std::marker::PhantomData,
        })
    }
    fn check(&self, path: &Path) -> Result<(), HostError> {
        if self.paths.contains(&path.canonicalize()?) {
            Ok(())
        } else {
            Err("native path was not explicitly trusted".into())
        }
    }
    pub fn load_clap(&self, product: &Product) -> Result<ClapInstance, HostError> {
        self.check(&product.path)?;
        if product.format != PluginFormat::Clap {
            return Err("requires CLAP product".into());
        }
        // SAFETY: capability authorizes this module and its dependencies.
        unsafe { ClapLibrary::load(&product.path) }?.instantiate(&product.id)
    }
    pub fn resolve_clap(
        &self,
        session: &crate::session::Session,
        missing: crate::session::MissingPolicy,
    ) -> Result<crate::session::ResolvedSession, HostError> {
        for slot in &session.slots {
            if slot.entry.path.exists() {
                self.check(&slot.entry.path)?;
            } else if matches!(missing, crate::session::MissingPolicy::Fail) {
                return Err("missing trusted CLAP path".into());
            }
        }
        // SAFETY: all present native paths were explicitly selected when this token was constructed.
        unsafe { session.resolve(missing) }
    }
    #[cfg(feature = "isolation")]
    /// The separately supplied helper is trusted by the caller; this token authorizes the native module.
    /// # Safety
    /// The helper executable and its dependencies must be trusted for this owner's lifetime.
    pub unsafe fn load_isolated_vst3(
        &self,
        product: &Product,
        spec: ProcessSpec,
        config: soniclaw_vst3::isolation::Config,
    ) -> Result<soniclaw_vst3::isolation::Isolated, HostError> {
        self.check(&product.path)?;
        if product.format != PluginFormat::Vst3 {
            return Err("requires VST3 product".into());
        }
        // SAFETY: selected module is authorized by this token; helper trust is the caller's contract.
        unsafe {
            soniclaw_vst3::isolation::Isolated::load(&product.path, Some(&product.id), spec, config)
        }
    }
    #[cfg(feature = "vst3")]
    pub fn load_vst3_buses(
        &self,
        product: &Product,
        spec: ProcessSpec,
    ) -> Result<soniclaw_vst3::buses::BusInstance, HostError> {
        self.check(&product.path)?;
        if product.format != PluginFormat::Vst3 {
            return Err("requires VST3 product".into());
        }
        // SAFETY: continued native-module trust and main-thread ownership belong to this capability.
        unsafe { soniclaw_vst3::buses::BusInstance::load(&product.path, Some(&product.id), spec) }
    }
    #[cfg(feature = "vst3")]
    pub fn load_vst3(
        &self,
        product: &Product,
        spec: ProcessSpec,
    ) -> Result<soniclaw_vst3::Instance, HostError> {
        self.check(&product.path)?;
        // SAFETY: this capability asserts continued trust and GUI-thread ownership.
        unsafe { soniclaw_vst3::Instance::load(&product.path, Some(&product.id), spec) }
    }
}
