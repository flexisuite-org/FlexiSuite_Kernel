use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use anyhow::Result;
use async_trait::async_trait;
use deno_core::{
    op2, v8, JsRuntime, OpState, RuntimeOptions as DenoOptions,
};
use std::sync::Once;

static INIT_V8: Once = Once::new();

pub struct DenoSandbox {
    options: RuntimeOptions,
}

impl DenoSandbox {
    pub fn new(options: RuntimeOptions) -> Self {
        Self { options }
    }
}

#[derive(Clone, Default)]
struct OutputState {
    value: serde_json::Value,
}

#[op2(fast)]
pub fn op_set_output(
    state: &mut OpState,
    #[string] json: String,
) {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
        state.put(OutputState { value });
    }
}

deno_core::extension!(
    sandbox_ext,
    ops = [op_set_output],
);

#[async_trait]
impl SandboxRuntime for DenoSandbox {
    async fn execute(
        &mut self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        let options = self.options.clone();
        let code = code.to_string();

        let result = tokio::task::spawn_blocking(move || {
            INIT_V8.call_once(|| { });

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| SandboxError::InitError(e.to_string()))?;

            rt.block_on(async move {
                // Use init() as determined by compiler
                let ext = sandbox_ext::init();

                let create_params = v8::CreateParams::default()
                    .heap_limits(0, options.memory_limit);

                let mut js_runtime = JsRuntime::new(DenoOptions {
                    extensions: vec![ext],
                    create_params: Some(create_params),
                    ..Default::default()
                });

                let isolate_handle = js_runtime.v8_isolate().thread_safe_handle();
                let wall_clock_limit = options.wall_clock_limit;

                std::thread::spawn(move || {
                    std::thread::sleep(wall_clock_limit);
                    isolate_handle.terminate_execution();
                });

                let input_json = serde_json::to_string(&input).unwrap();
                let setup_code = format!("globalThis.INPUT = {};", input_json);
                js_runtime
                    .execute_script("setup", setup_code)
                    .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

                let wrapped_code = format!(r#"
                    (function() {{
                        const result = eval({});
                        Deno.core.ops.op_set_output(JSON.stringify(result));
                    }})()
                "#, serde_json::to_string(&code).unwrap());

                let execution_future = async {
                    let result = js_runtime.execute_script("user_code", wrapped_code);
                    match result {
                        Ok(global) => {
                            let result = js_runtime.resolve(global).await;
                            match result {
                                Ok(_) => Ok(()),
                                Err(e) => Err(e),
                            }
                        }
                        Err(e) => Err(e),
                    }
                };

                match tokio::time::timeout(options.wall_clock_limit, execution_future).await {
                    Ok(res) => match res {
                        Ok(_) => {
                            let op_state = js_runtime.op_state();
                            let op_state = op_state.borrow();
                            if let Some(output) = op_state.try_borrow::<OutputState>() {
                                Ok(output.value.clone())
                            } else {
                                Ok(serde_json::Value::Null)
                            }
                        }
                        Err(e) => Err(SandboxError::RuntimeError(e.to_string())),
                    },
                    Err(_) => Err(SandboxError::Timeout),
                }
            })
        })
        .await;

        match result {
            Ok(res) => res,
            Err(e) => Err(SandboxError::RuntimeError(e.to_string())),
        }
    }
}
