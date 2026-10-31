use std::rc::Rc;

use collections::{HashMap, IndexMap};
use gpui::{App, AppContext as _, Context, Entity, Global, SharedString, Task};

use crate::{AgentId, Project};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAgentModel {
    pub id: SharedString,
    pub name: SharedString,
    pub group: Option<SharedString>,
}

#[derive(Debug)]
pub enum ExternalAgentModelsState {
    Loading,
    Ready(Vec<ExternalAgentModel>),
    AuthRequired,
    Failed(SharedString),
}

/// App-wide catalog of the models exposed by installed external agents, keyed
/// by `AgentId`. A single catalog serves every project and window. Entries
/// only change when a probe or an open-thread read writes them; the catalog
/// does not track agent installs, removals, or settings changes.
pub struct ExternalAgentModelCatalog {
    states: IndexMap<AgentId, ExternalAgentModelsState>,
    tasks: HashMap<AgentId, Task<()>>,
}

impl ExternalAgentModelCatalog {
    fn new() -> Self {
        Self {
            states: IndexMap::default(),
            tasks: HashMap::default(),
        }
    }

    pub fn state(&self, agent_id: &AgentId) -> Option<&ExternalAgentModelsState> {
        self.states.get(agent_id)
    }

    pub fn set_state(
        &mut self,
        agent_id: AgentId,
        state: ExternalAgentModelsState,
        cx: &mut Context<Self>,
    ) {
        self.states.insert(agent_id, state);
        cx.notify();
    }

    pub fn set_task(&mut self, agent_id: AgentId, task: Task<()>) {
        self.tasks.insert(agent_id, task);
    }
}

type ExternalAgentModelSource =
    Rc<dyn Fn(Entity<Project>, Entity<ExternalAgentModelCatalog>, &mut App)>;

#[derive(Default)]
struct GlobalExternalAgentModelCatalogs {
    catalog: Option<Entity<ExternalAgentModelCatalog>>,
    source: Option<ExternalAgentModelSource>,
}

impl Global for GlobalExternalAgentModelCatalogs {}

fn model_source(cx: &App) -> Option<ExternalAgentModelSource> {
    cx.try_global::<GlobalExternalAgentModelCatalogs>()
        .and_then(|catalogs| catalogs.source.clone())
}

/// Registers the source responsible for populating the catalog with the
/// models exposed by installed external agents. Called once by the crate that
/// can create agent sessions (currently `agent_ui`).
pub fn set_external_agent_model_source(
    source: impl Fn(Entity<Project>, Entity<ExternalAgentModelCatalog>, &mut App) + 'static,
    cx: &mut App,
) {
    cx.default_global::<GlobalExternalAgentModelCatalogs>()
        .source = Some(Rc::new(source));
}

/// Returns the app-wide catalog if one has been created. Never invokes the
/// source.
pub fn external_agent_model_catalog(cx: &App) -> Option<Entity<ExternalAgentModelCatalog>> {
    cx.try_global::<GlobalExternalAgentModelCatalogs>()?
        .catalog
        .clone()
}

/// Returns the app-wide catalog, creating it on first use. Never invokes the
/// source.
pub fn external_agent_model_catalog_or_init(cx: &mut App) -> Entity<ExternalAgentModelCatalog> {
    let existing = cx
        .default_global::<GlobalExternalAgentModelCatalogs>()
        .catalog
        .clone();
    match existing {
        Some(catalog) => catalog,
        None => {
            let catalog = cx.new(|_cx| ExternalAgentModelCatalog::new());
            cx.global_mut::<GlobalExternalAgentModelCatalogs>().catalog = Some(catalog.clone());
            catalog
        }
    }
}

/// Returns the app-wide catalog, creating it on first use, and asks the
/// registered source to refresh it. Returns `None` when no source has been
/// registered.
pub fn refresh_external_agent_models(
    project: &Entity<Project>,
    cx: &mut App,
) -> Option<Entity<ExternalAgentModelCatalog>> {
    let source = model_source(cx)?;
    let catalog = external_agent_model_catalog_or_init(cx);
    source(project.clone(), catalog.clone(), cx);
    Some(catalog)
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use fs::FakeFs;
    use gpui::{TestAppContext, UpdateGlobal as _};
    use settings::SettingsStore;

    use crate::AgentServersUpdated;

    use super::*;

    fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
        });
    }

    fn install_custom_agent(id: &str, cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |content| {
                    content.agent_servers.get_or_insert_default().insert(
                        id.to_string(),
                        settings::CustomAgentServerSettings::Custom {
                            path: PathBuf::from("/usr/bin/fake-agent"),
                            args: Vec::new(),
                            env: Default::default(),
                            default_mode: None,
                            default_config_options: Default::default(),
                            favorite_config_option_values: Default::default(),
                        },
                    );
                });
            });
        });
    }

    fn uninstall_agent(id: &str, cx: &mut TestAppContext) {
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |content| {
                    content.agent_servers.get_or_insert_default().0.remove(id);
                });
            });
        });
    }

    fn emit_agent_servers_updated(project: &Entity<Project>, cx: &mut TestAppContext) {
        let agent_server_store =
            project.read_with(cx, |project, _cx| project.agent_server_store().clone());
        agent_server_store.update(cx, |_store, cx| cx.emit(AgentServersUpdated));
        cx.run_until_parked();
    }

    fn state_is(
        catalog: &Entity<ExternalAgentModelCatalog>,
        agent_id: &str,
        cx: &mut TestAppContext,
    ) -> Option<String> {
        catalog.read_with(cx, |catalog, _cx| {
            catalog
                .state(&AgentId::new(agent_id))
                .map(|state| match state {
                    ExternalAgentModelsState::Loading => "loading".to_string(),
                    ExternalAgentModelsState::Ready(_) => "ready".to_string(),
                    ExternalAgentModelsState::AuthRequired => "auth_required".to_string(),
                    ExternalAgentModelsState::Failed(_) => "failed".to_string(),
                })
        })
    }

    #[gpui::test]
    async fn no_source_returns_none(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;

        cx.update(|cx| {
            assert!(refresh_external_agent_models(&project, cx).is_none());
            assert!(external_agent_model_catalog(cx).is_none());
        });
    }

    #[gpui::test]
    async fn source_is_called_once_per_refresh_and_access_does_not_invoke_it(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let call_count = Arc::new(AtomicUsize::new(0));
        let notifications = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            set_external_agent_model_source(
                {
                    let call_count = call_count.clone();
                    move |_project, catalog, cx| {
                        call_count.fetch_add(1, Ordering::SeqCst);
                        catalog.update(cx, |catalog, cx| {
                            catalog.set_state(
                                AgentId::new("agent-one"),
                                ExternalAgentModelsState::Loading,
                                cx,
                            );
                        });
                    }
                },
                cx,
            );
        });

        let catalog = cx
            .update(|cx| refresh_external_agent_models(&project, cx))
            .expect("catalog should be returned once a source is registered");
        assert_eq!(call_count.load(Ordering::SeqCst), 1);

        let subscription = cx.update(|cx| {
            cx.observe(&catalog, {
                let notifications = notifications.clone();
                move |_catalog, _cx| {
                    notifications.fetch_add(1, Ordering::SeqCst);
                }
            })
        });

        let accessed = cx
            .update(|cx| external_agent_model_catalog(cx))
            .expect("catalog lookup should reuse the stored catalog");
        assert_eq!(accessed, catalog);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        assert!(catalog.read_with(cx, |catalog, _cx| {
            matches!(
                catalog.state(&AgentId::new("agent-one")),
                Some(ExternalAgentModelsState::Loading)
            )
        }));

        cx.update(|cx| {
            catalog.update(cx, |catalog, cx| {
                catalog.set_state(
                    AgentId::new("agent-one"),
                    ExternalAgentModelsState::Ready(vec![]),
                    cx,
                );
            });
        });
        cx.update(|cx| {
            catalog.update(cx, |catalog, cx| {
                catalog.set_state(
                    AgentId::new("agent-two"),
                    ExternalAgentModelsState::Failed("boom".into()),
                    cx,
                );
            });
        });
        assert_eq!(notifications.load(Ordering::SeqCst), 2);

        cx.update(|cx| {
            refresh_external_agent_models(&project, cx);
        });
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert_eq!(notifications.load(Ordering::SeqCst), 3);
        drop(subscription);
    }

    #[gpui::test]
    async fn catalog_is_app_wide_and_shared_across_projects(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        let project_one = Project::test(fs.clone(), [], cx).await;
        let project_two = Project::test(fs, [], cx).await;
        let call_count = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            set_external_agent_model_source(
                {
                    let call_count = call_count.clone();
                    move |_project, catalog, cx| {
                        call_count.fetch_add(1, Ordering::SeqCst);
                        catalog.update(cx, |catalog, cx| {
                            catalog.set_state(
                                AgentId::new("agent-one"),
                                ExternalAgentModelsState::Ready(vec![]),
                                cx,
                            );
                        });
                    }
                },
                cx,
            );
        });

        let catalog = cx
            .update(|cx| refresh_external_agent_models(&project_one, cx))
            .expect("catalog");
        let same_catalog = cx
            .update(|cx| refresh_external_agent_models(&project_two, cx))
            .expect("catalog");
        assert_eq!(catalog, same_catalog);
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
        assert_eq!(
            state_is(&catalog, "agent-one", cx).as_deref(),
            Some("ready")
        );

        // AgentServersUpdated does not invalidate catalog state; entries only
        // change when a probe or open-thread read writes them.
        emit_agent_servers_updated(&project_two, cx);
        assert_eq!(
            state_is(&catalog, "agent-one", cx).as_deref(),
            Some("ready")
        );
    }

    #[gpui::test]
    async fn agent_servers_updated_does_not_change_states_or_call_source(cx: &mut TestAppContext) {
        init_test(cx);
        install_custom_agent("agent-one", cx);
        install_custom_agent("agent-two", cx);
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        cx.run_until_parked();
        let call_count = Arc::new(AtomicUsize::new(0));

        cx.update(|cx| {
            set_external_agent_model_source(
                {
                    let call_count = call_count.clone();
                    move |_project, catalog, cx| {
                        call_count.fetch_add(1, Ordering::SeqCst);
                        catalog.update(cx, |catalog, cx| {
                            catalog.set_state(
                                AgentId::new("agent-one"),
                                ExternalAgentModelsState::Ready(vec![]),
                                cx,
                            );
                            catalog.set_state(
                                AgentId::new("agent-two"),
                                ExternalAgentModelsState::Loading,
                                cx,
                            );
                            catalog.set_task(AgentId::new("agent-two"), cx.spawn(async |_, _| {}));
                        });
                    }
                },
                cx,
            );
        });
        let catalog = cx
            .update(|cx| refresh_external_agent_models(&project, cx))
            .expect("catalog");

        emit_agent_servers_updated(&project, cx);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        catalog.read_with(cx, |catalog, _cx| {
            assert!(matches!(
                catalog.state(&AgentId::new("agent-one")),
                Some(ExternalAgentModelsState::Ready(_))
            ));
            assert!(matches!(
                catalog.state(&AgentId::new("agent-two")),
                Some(ExternalAgentModelsState::Loading)
            ));
            assert!(catalog.tasks.contains_key(&AgentId::new("agent-two")));
        });

        // Even removing an agent mid-session leaves its catalog entry alone;
        // the next application start's preload re-evaluates installed agents.
        uninstall_agent("agent-one", cx);
        cx.run_until_parked();
        emit_agent_servers_updated(&project, cx);
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        catalog.read_with(cx, |catalog, _cx| {
            assert!(catalog.state(&AgentId::new("agent-one")).is_some());
            assert!(catalog.state(&AgentId::new("agent-two")).is_some());
        });
    }
}
