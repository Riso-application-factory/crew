# Fork Divergences

## Agent Profiles settings page (`settings_ui`)

**Purpose.** Expose the profile management of `ManageProfilesModal` as a
Settings child page under AI → Agent Configuration → Agent Profiles, reading
and writing the same `agent.profiles` / `agent.default_profile` user settings
schema. The modal, `agent::ManageProfiles`, and all upstream CRUD semantics are
unchanged.

**Touched symbols.**

- `crates/settings_ui/src/pages/agent_profiles_page.rs` — list/detail/form
  render functions, `AgentProfilePageState`, `AgentProfileForm`,
  `sync_agent_profile_page_state`, write helpers (`write_profile`,
  `set_builtin_tool_enabled`, `set_context_server_tool_enabled`,
  `set_enable_all_context_servers`, `set_profile_default_model`,
  `set_profile_external_default_model`, `clear_profile_default_model`,
  `delete_agent_profile`), `external_default_model_label`, telemetry guards
  (`mark_*_configured`).
- `crates/settings_ui/src/components/language_model_picker.rs` —
  `render_language_model_picker`, `PickerModel`, and
  `LanguageModelPickerDelegate` (Zed sources mirror `agent_ui`'s
  `LanguageModelSelector::all_models`/`update_matches`; the external section
  reads `project::external_agent_models`).
- `crates/settings_ui/src/page_data.rs` — `Agent Profiles` `SubPageLink` after
  `Tool Permissions` in `agent_configuration_section`.
- `crates/settings_ui/src/settings_ui.rs` — `agent_profile_page` /
  `agent_profile_form` fields, `is_agent_profiles_page` header detection,
  `Add Profile` button, one `sync_agent_profile_page_state` call in the
  `SettingsStore` observer.
- `crates/settings_ui/src/{pages.rs,components.rs,Cargo.toml}` — registration
  and the `convert_case` workspace dependency (same derivation as
  `AgentProfile::create`).

**Upstream behaviors preserved.**

- All reads use `AgentSettings::get_global(cx).profiles` / `AgentProfile::
  available_profiles`; all writes go through `settings::update_settings_file`
  and produce exactly the `AgentProfileContent` shape that
  `AgentProfileSettings::save_to_settings` and `ToolPickerDelegate::confirm`
  write (`name`, `tools`, `enable_all_context_servers`,
  `context_servers.<server>.tools.<tool>`, `default_model`).
- Create/fork use `AgentProfile::create` unchanged (kebab id, copied
  tools/MCP/default model). Delete resets `agent.default_profile` to `write`
  when the deleted profile was the default; built-ins are not deleted.
- Telemetry event names and fields match the modal:
  `Agent Profile Created` (`profile_id`, `is_fork`, `base_profile_id`),
  `Agent Profile Deleted` (`profile_id`), `Agent Profile Tools/MCPs/
  Default Model Configured` (`profile_id`, `is_builtin`).

**Intentional divergences (D1–D6).**

- D1: the list orders profiles as `AgentProfile::available_profiles`
  (built-ins Write/Ask/Minimal, then custom in settings order), not the
  modal's alphabetical sort.
- D2: MCP tool toggles write the new switch state computed via
  `is_context_server_tool_enabled`; the modal ToolPicker first-click defect
  (writes `!false` under `enable_all_context_servers = true`) is not
  reproduced.
- D3: selecting or clearing a default model on an unmodified built-in
  materializes the full profile first (the modal's write silently no-ops
  there — F5).
- D4: `Clear` removes `default_model` and `Enable All MCP Servers` toggles
  `enable_all_context_servers`; neither exists in the modal (F8).
- D5: each `… Configured` event fires at most once per detail-page opening,
  on the first write of that section; the modal fires on screen open.
- D6: the add/fork form rejects empty names, symbol-only names (kebab id has
  no alphanumeric) and duplicate ids; the modal accepts them.
- The built-in tools list is not filtered by
  `agent::tool_supports_provider` (the modal's `active_model = None` path):
  when a native thread has a model selected, the modal's tool picker hides
  provider-unsupported tools and this page shows them.
- The built-in one-line descriptions duplicate `ProfileSelector::
  documentation` to keep `settings_ui` independent of `agent_ui`.
- D7: the Default Model picker also lists models of installed external
  agents (after the Zed models, agents sorted by display name
  case-insensitively). `render_language_model_picker` takes a
  `current_selection: Option<LanguageModelSelection>` plus the active
  `project: Option<Entity<Project>>`, and its callback is
  `Fn(PickerModel, &mut App)` — `PickerModel::Zed(LanguageModel)` keeps the
  old write path; `PickerModel::External { agent_id, agent_name, model }`
  writes `provider = <AgentId>`, `model = <value id>`,
  `enable_thinking = false` with no `effort`/`speed` (AD-11). No provider is
  registered under an `AgentId`, so native `resolve_model_from_selection`
  skips the value and the native thread keeps its model.
- Agents whose `AgentId` collides with a registered provider id (e.g.
  `zed.dev`, `anthropic`) are not listed; `Loading`, `Not signed in`
  (`AuthRequired`) and `Couldn't load models` (`Failed`) render as
  non-selectable status rows, and `Ready(vec![])` renders nothing.
- The catalog is refreshed only when the picker opens
  (`refresh_external_agent_models` inside the delegate); the trigger label
  and page renders read `external_agent_model_catalog` only and never
  probe. Reopening the picker re-highlights the row matching
  `current_selection` even when the model arrives after the picker is
  open.
- `ZED_AGENT_ONLY` is kept deliberately: external `default_model` values
  are persisted but nothing applies them to external sessions yet, so the
  note stays accurate (DESIGN §5).

**Verification.**

- `cargo test -p settings_ui` — 100 tests: the original
  `pages::agent_profiles_page::tests` plus 7 new picker tests in
  `components::language_model_picker::tests` (display-name ordering,
  `Ready(vec![])`/no-state rows, provider-id collision, non-selectable
  status rows + no dismiss on confirm, query preserved across catalog
  changes, late-arriving selection highlight, no-source/no-project
  Zed-only) and 2 new page tests (external JSON write shape,
  trigger-label catalog read without probing).
- `./script/clippy` — clean.
- Prettier check on `docs/src/ai/agent-profiles.md` — clean.

**Upstream revision last verified:** `83323324a4`.

## External-agent model catalog (`project` + `agent_ui`)

**Purpose.** App-wide, in-memory catalog of the models exposed by each
installed external agent, keyed by `AgentId`. `project` owns the
dependency-free contract so consumers such as `settings_ui` never depend on
`agent_ui`; `agent_ui` registers a source that populates the catalog from
open `AgentPanel` sessions or short-lived probe sessions.

**Touched symbols.**

- `crates/project/src/external_agent_models.rs` — `ExternalAgentModel`,
  `ExternalAgentModelsState`, `ExternalAgentModelCatalog`
  (`state`/`set_state`/`set_task`), `set_external_agent_model_source`,
  `external_agent_model_catalog`, `external_agent_model_catalog_or_init`,
  `refresh_external_agent_models`; a private
  `GlobalExternalAgentModelCatalogs` holding the single app-wide catalog.
- `crates/agent_ui/src/external_agent_models.rs` — `init` (one call from
  `agent_ui::init`), the source `refresh`/`refresh_with_panel`,
  `ExternalAgentModelPreload` (startup preload state),
  `maybe_start_preload`/`wait_for_registry`/`on_registry_notification`/
  `start_preload` (`App::observe_new::<AgentPanel>` trigger),
  `find_agent_panel` (`workspace::AppState` workspace scan),
  `probe_agent`/`run_probe`/`run_probe_session`/`read_models`/
  `probe_session_needs_archive`/`archive_probe_session` helpers,
  `PROBE_SESSION_TITLE`, and the `CatalogTestConnection`/
  `CatalogTestConfigOptions`/`CatalogTestModelSelector`/
  `CatalogTestSessionList`/`GatedFetchHttpClient` test types.
- `crates/agent_ui/src/threads_archive_view.rs` —
  `is_probe_session_metadata`, applied in `update_items` and in the
  `archived_entries()` checks (`update_items` filter fallback and
  `render_toolbar`'s `has_archived_threads`).

**Upstream behaviors preserved.**

- Connections are only ever obtained through the panel's
  `AgentConnectionStore::request_connection`; no external-agent process is
  spawned independently, and `Agent::server`/`CustomAgentServer` keep owning
  command construction.
- `AuthRequired::from_error` remains the single classifier for auth errors;
  the probe never calls `authenticate`.
- Reading the catalog (`external_agent_model_catalog`) never invokes the
  source, so renders cannot start agent processes.

**Intentional divergences.**

- The catalog is app-wide: one `ExternalAgentModelCatalog` serves every
  project and window. Entries change only when a probe or an open-thread
  read writes them; the catalog does not observe `AgentServersUpdated` and
  does not track installs, removals, or settings changes.
- Refresh happens only at three points: the startup preload, opening the
  profile detail/model picker for an agent with no result or a `Failed`
  result, and reading models from an already-open external-agent thread.
  `Ready`/`AuthRequired`/`Loading` entries are never reprobed; `Failed` is
  retried on the next refresh.
- Refresh is a no-op — no probes and no catalog writes — when the project
  is shared/collab, when no `AgentPanel` exists for the project, or when
  `DisableAiSettings` is on. Existing entries stay as they are. Remote
  projects are not special-cased: they are still probed through the picker.
- On startup the catalog preloads once, the first time a local
  `AgentPanel` exists while AI is enabled: `observe_new::<AgentPanel>`
  runs inside the panel's own `entity.update`, where reading the panel
  entity would panic, so the callback only inspects the passed `&mut`
  panel (and other entities) and defers the rest via `App::defer`. The
  deferred step re-checks `AgentRegistryStore::is_fetching`; while the
  initial fetch is still running the panel is recorded as the candidate
  and a single `App::observe` subscription waits for the fetch-finished
  notification, which defers again into `start_preload`. A registry store
  that never existed takes the same deferred path and starts immediately.
  A dropped candidate panel retries through the next `AgentPanel`.
- Probing an agent with no open session creates a throwaway ACP session
  (no `ConversationView`, sidebar entry, or visible `ThreadMetadataStore`
  row) to read its model list. The whole probe — the pending connection
  wait plus `new_session` — is bounded by one 30s executor-timer timeout; a
  late-resolving session is closed but its result is discarded, and a
  timed-out `Failed` entry is retried on the next refresh against the same
  pending `AgentConnectionStore` entry.
- Probe-session cleanup: `close_session` when `supports_close_session`,
  then `delete_session` when the agent's session list exists and supports
  deletion. No metadata is written when the agent has no session list (the
  session can never surface in Import Thread) or when deletion succeeds.
  A `ThreadMetadata` row with `archived: true` and title
  `PROBE_SESSION_TITLE` is written only when a session list exists and the
  session cannot be deleted (`delete_session` unsupported or failed), so
  the leftover session stays out of the sidebar and Import Thread.
- `ThreadsArchiveView` filters archived probe rows (`title ==
  PROBE_SESSION_TITLE`) out of `update_items` and out of both
  "has archived threads" checks, so persisted probe rows stay in storage
  (old rows must not be deleted — deleting them would let the session
  resurface in Import Thread) but never appear in the Archive list.
- **Approved limitation:** cataloged external models are not applied to
  external sessions. The Agent Profiles settings page persists the
  selection through the existing `LanguageModelSelection` `default_model`
  profile field with `provider = <AgentId>`; nothing reads that field for
  external agents yet.
- **Approved limitation (F8):** an agent whose settings, registry version,
  or installation changes mid-session keeps its stale catalog entry (and
  any persisted probe-session metadata rows may remain in history) until
  the next application start re-runs the preload.

**Upstream code to re-check on upstream updates.**

- `AgentConnectionStore::request_connection` — the catalog relies on
  pending-entry reuse for timeout retries.
- `ConversationView::initial_state` — `is_via_collab` gating,
  `default_path_list` work dirs, and `AuthRequired::from_error`
  classification must keep matching how sessions are normally opened.
- The config-option vs `model_selector` precedence in `ConversationView`
  (around `:1636-1712`).
- `config_options.rs` — how the `Model` category `Select` option is
  located (`first_config_option_id_matching`).
- `AgentModelList` / `AgentModelInfo.disabled` semantics.
- `AgentConnection::close_session` / `AgentSessionList::delete_session`
  capability checks.
- `thread_import.rs` — where leftover probe sessions appear in
  history/import listings.
- `ThreadsArchiveView::update_items` and its `archived_entries()` checks —
  upstream changes to how archived rows are listed or counted must keep
  the `is_probe_session_metadata` filter intact.

**Verification.**

- `cargo test -p project external_agent_models` — 4 tests (no source
  returns none, single source call + catalog notifications, app-wide
  sharing across projects, `AgentServersUpdated` ignored incl. agent
  removal).
- `cargo test -p agent_ui external_agent_models` — 28 tests (config-option
  and selector extraction, auth classification, connect/session timeout +
  late-session cleanup, loading-skip, open-session reuse without
  `session/new`, stable/`Failed` retry rules, `AgentServersUpdated`
  mid-probe and steady-state being ignored, close + delete cleanup,
  archive on undeletable or failed delete, no metadata without a session
  list, no UI/session mutation, collab / no-panel / `disable_ai` refresh
  no-ops preserving state, preload once-per-run, preload waiting on the
  registry fetch, preload on a ready or absent registry, `disable_ai` /
  non-local / dropped-panel preload skips).
- `cargo test -p agent_ui threads_archive_view` — probe rows hidden under
  both filters; probe-only storage renders the empty state and falls back
  to the All filter.
- `./script/clippy` — clean.

**Upstream revision last verified:** `83323324a4`.
