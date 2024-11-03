use mlua::prelude::*;
use thiserror::Error;

use std::collections::HashSet;

use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("lua error: {0}")]
    LuaError(LuaError),

    #[error("failed to load plugin: {0}")]
    FailedToLoadPlugin(std::io::Error),
}

pub struct LuaNgmpApi {
    loaded_plugins: HashSet<String>,
}

impl LuaNgmpApi {
    pub fn new() -> Self {
        Self {
            loaded_plugins: HashSet::new(),
        }
    }
}

/// The Lua environment houses the global Lua state and manages (re)loading plugins and calling functions.
pub struct LuaEnvironment {
    lua: Lua,

    ngmp_api: Arc<Mutex<LuaNgmpApi>>,
    loaded_plugins: HashSet<String>,
}

impl LuaEnvironment {
    pub fn new() -> LuaResult<Self> {
        let mut s = Self {
            lua: Lua::new(),
            ngmp_api: Arc::new(Mutex::new(LuaNgmpApi::new())),
            loaded_plugins: HashSet::new(),
        };

        s.init_lua_env()?;

        Ok(s)
    }

    fn init_lua_env(&mut self) -> LuaResult<()> {
        // Define the `ngmp` api
        let ngmp_api_table = self.lua.create_table()?;

        let get_plugins_fn = {
            let api_ref = self.ngmp_api.clone();
            self.lua.create_async_function(move |_lua: Lua, _: ()| {
                let api = api_ref.clone();
                async move {
                    let lock = api.lock().await;
                    let plugins_vec = lock
                        .loaded_plugins
                        .iter()
                        .map(|s| s.clone())
                        .collect::<Vec<String>>();
                    Ok(plugins_vec)
                }
            })?
        };
        ngmp_api_table.set("get_plugins", get_plugins_fn)?;

        // Define our custom print function
        let print_fn = self.lua.create_function(|_lua, (msg,): (String,)| {
            info!("[LUA] {}", msg);
            Ok(())
        })?;

        // Set up the globals
        let globals = self.lua.globals();
        globals.set("ngmp", ngmp_api_table)?;
        globals.set("print", print_fn)?;

        Ok(())
    }

    pub async fn load_plugin<P: AsRef<std::path::Path>>(
        &mut self,
        plugin_name: String,
        path: P,
    ) -> Result<(), PluginError> {
        let path = path.as_ref();

        let lua_code =
            std::fs::read_to_string(path).map_err(|e| PluginError::FailedToLoadPlugin(e))?;

        let plugin: LuaTable = self
            .lua
            .load(lua_code)
            .set_name(&plugin_name)
            .eval()
            .map_err(|e| PluginError::LuaError(e))?;

        self.lua
            .globals()
            .set(plugin_name.as_str(), plugin)
            .map_err(|e| PluginError::LuaError(e))?;

        {
            let mut lock = self.ngmp_api.lock().await;
            lock.loaded_plugins.insert(plugin_name.clone());
        }

        self.loaded_plugins.insert(plugin_name.clone());

        let _: Option<()> = self
            .call_async_fn(&plugin_name, "onPluginLoad", ())
            .await
            .map_err(|e| PluginError::LuaError(e))?;

        Ok(())
    }

    pub async fn call_async_fn<A: IntoLuaMulti, T: FromLuaMulti>(
        &self,
        plugin_name: &str,
        func_name: &str,
        args: A,
    ) -> LuaResult<Option<T>> {
        let plugin: LuaTable = self.lua.globals().get(plugin_name)?;
        if let Ok(func) = plugin.get::<LuaFunction>(func_name) {
            Ok(Some(func.call_async(args).await?))
        } else {
            Ok(None)
        }
    }

    pub async fn event_on_player_auth(&self, steam_id: u64, name: String) {
        let sid = steam_id.to_string();
        for plugin_name in &self.loaded_plugins {
            let res: LuaResult<Option<()>> = self
                .call_async_fn(&plugin_name, "onPlayerAuth", (sid.clone(), name.clone()))
                .await;

            if let Err(e) = res {
                error!("[LUA] {}", e);
            }
        }
    }
}
