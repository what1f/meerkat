//! Non-production JavaScript mock using the same host as shipped adapters.
use lankide_harness::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct ProcessHarnessSpec {
    pub arguments: Vec<String>,
    pub bind_timeout: Duration,
    pub cancel_grace: Duration,
    pub close_grace: Duration,
    pub pid_journal: Option<PathBuf>,
}
impl Default for ProcessHarnessSpec {
    fn default() -> Self {
        Self {
            arguments: vec![],
            bind_timeout: Duration::from_secs(5),
            cancel_grace: Duration::from_millis(250),
            close_grace: Duration::from_millis(250),
            pid_journal: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProcessHarnessFactory {
    plugin: JsHarnessFactory,
}
impl ProcessHarnessFactory {
    pub fn new(info: HarnessInfo, spec: ProcessHarnessSpec) -> Self {
        let options = serde_json::json!({"id":info.harness_id,"name":info.display_name,"arguments":spec.arguments,"bind_timeout":spec.bind_timeout.as_millis(),"cancel_grace":spec.cancel_grace.as_millis(),"close_grace":spec.close_grace.as_millis()});
        let source = format!("const mockOptions={options};\n{}", include_str!("mock.js"));
        let mut plugin = JsHarnessFactory::new(&info.harness_id, &info.display_name, source)
            .expect("valid mock plugin");
        if let Some(path) = spec.pid_journal {
            plugin = plugin.with_pid_journal(path);
        }
        Self { plugin }
    }
}
impl HarnessFactory for ProcessHarnessFactory {
    fn info(&self) -> &HarnessInfo {
        self.plugin.info()
    }
    fn probe<'a>(&'a self, c: &'a HarnessConfig) -> HarnessFuture<'a, ProbeResult> {
        self.plugin.probe(c)
    }
    fn models<'a>(&'a self, c: &'a HarnessConfig) -> HarnessFuture<'a, Vec<NativeModel>> {
        self.plugin.models(c)
    }
    fn start<'a>(&'a self, r: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        self.plugin.start(r)
    }
    fn start_before<'a>(
        &'a self,
        r: SessionRequest,
        d: Instant,
    ) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        self.plugin.start_before(r, d)
    }
    fn resume<'a>(&'a self, r: SessionRequest) -> HarnessFuture<'a, Box<dyn HarnessSession>> {
        self.plugin.resume(r)
    }
}
