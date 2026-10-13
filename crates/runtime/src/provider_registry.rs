use super::*;

pub trait ProviderTypeFactory: Send + Sync {
    fn info(&self) -> ProviderInfo;
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }
    fn create(
        &self,
        workspace: &WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn ProviderHostInstance>, ProviderError>;
    fn workspace_onboarding(
        &self,
        _action: String,
        _opaque_payload: serde_json::Value,
        _services: HostServices,
    ) -> ProviderFactoryFuture<'_, ProviderOnboarding> {
        Box::pin(async {
            Err(ProviderError::Unsupported {
                operation: "workspace_onboarding",
            })
        })
    }
}

pub type ProviderFactoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

#[derive(Clone, Default)]
pub struct ProviderFactoryRegistry {
    factories: Arc<StdRwLock<BTreeMap<ProviderId, Arc<dyn ProviderTypeFactory>>>>,
}
impl ProviderFactoryRegistry {
    pub fn register(&self, factory: Arc<dyn ProviderTypeFactory>) -> Result<(), RuntimeError> {
        let id = factory.info().id;
        let mut factories = self
            .factories
            .write()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?;
        if factories.insert(id.clone(), factory).is_some() {
            return Err(RuntimeError::DuplicateProviderType(id));
        }
        Ok(())
    }

    pub fn register_host_factory<P, F>(
        &self,
        info: ProviderInfo,
        factory: F,
    ) -> Result<(), RuntimeError>
    where
        P: Provider,
        F: Fn(&WorkspaceRecord, HostServices) -> Result<ProviderHost<P>, ProviderError>
            + Send
            + Sync
            + 'static,
    {
        self.register(Arc::new(HostFactory {
            info,
            factory,
            _provider: PhantomData,
        }))
    }

    pub fn factory(
        &self,
        provider_id: &ProviderId,
    ) -> Result<Arc<dyn ProviderTypeFactory>, RuntimeError> {
        self.factories
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .get(provider_id)
            .cloned()
            .ok_or_else(|| RuntimeError::UnknownProviderType(provider_id.clone()))
    }

    pub fn catalog(&self) -> Result<Vec<ProviderInfo>, RuntimeError> {
        Ok(self
            .factories
            .read()
            .map_err(|_| RuntimeError::RuntimeLockPoisoned)?
            .values()
            .map(|factory| factory.info())
            .collect())
    }
}

struct HostFactory<P, F> {
    info: ProviderInfo,
    factory: F,
    _provider: PhantomData<fn() -> P>,
}
impl<P, F> ProviderTypeFactory for HostFactory<P, F>
where
    P: Provider,
    F: Fn(&WorkspaceRecord, HostServices) -> Result<ProviderHost<P>, ProviderError> + Send + Sync,
{
    fn info(&self) -> ProviderInfo {
        self.info.clone()
    }
    fn capabilities(&self) -> Vec<String> {
        P::capabilities()
            .iter()
            .map(|capability| (*capability).to_owned())
            .collect()
    }
    fn create(
        &self,
        workspace: &WorkspaceRecord,
        services: HostServices,
    ) -> Result<Arc<dyn ProviderHostInstance>, ProviderError> {
        (self.factory)(workspace, services)
            .map(|host| Arc::new(host) as Arc<dyn ProviderHostInstance>)
    }
}
