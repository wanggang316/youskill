// Services module - business logic layer
// Commands should only handle Tauri API and delegate to services

pub mod agent_apps_service;
pub mod ai_service;
pub mod avatar_service;
pub mod diff_service;
pub mod drift_service;
pub mod env;
pub mod github_auth_service;
pub mod hub_service;
pub mod install_service;
pub mod instruction_service;
pub mod library_sync_service;
pub mod lock_service;
pub mod migration_service;
pub mod remote_service;
pub mod scan_roots_service;
pub mod scan_service;
pub mod skill_service;
pub mod source_service;
pub mod translate_service;
pub mod user_projects_service;
pub mod workspace_service;
