//! Process-local bytecode for fixed embedded libraries, with fresh JS state on every eval.
use crate::profile;
use rquickjs::{Ctx, Error, Value, qjs};
use std::{ffi::CString, slice, sync::OnceLock};

pub(crate) struct Script {
    source: &'static [u8],
    bytecode: OnceLock<Vec<u8>>,
}

impl Script {
    const fn new(source: &'static [u8]) -> Self {
        Self {
            source,
            bytecode: OnceLock::new(),
        }
    }

    pub(crate) fn eval(&self, ctx: &Ctx<'_>) -> rquickjs::Result<()> {
        let raw = ctx.as_raw().as_ptr();
        let compiled = if let Some(bytes) = self.bytecode.get() {
            let _load = profile::span("bootstrap_load");
            // SAFETY: Only this binary's pinned engine writes these private, immutable
            // bytes. No external bytecode or JS handles enter the cache. Without
            // ROM_DATA the engine copies the buffer into the current runtime.
            unsafe {
                owned(
                    ctx,
                    qjs::JS_ReadObject(
                        raw,
                        bytes.as_ptr(),
                        bytes.len() as _,
                        qjs::JS_READ_OBJ_BYTECODE as _,
                    ),
                )?
            }
        } else {
            let _compile = profile::span("bootstrap_compile");
            let source = CString::new(self.source)?;
            // SAFETY: Source and filename are NUL-terminated and live for the call.
            // Global/strict/default filename match Ctx::eval in rquickjs 0.14.0.
            // COMPILE_ONLY does not execute or retain the compiling context's globals.
            let compiled = unsafe {
                owned(
                    ctx,
                    qjs::JS_Eval(
                        raw,
                        source.as_ptr(),
                        self.source.len() as _,
                        c"eval_script".as_ptr(),
                        (qjs::JS_EVAL_TYPE_GLOBAL
                            | qjs::JS_EVAL_FLAG_STRICT
                            | qjs::JS_EVAL_FLAG_COMPILE_ONLY) as _,
                    ),
                )?
            };
            let mut length = 0;
            // SAFETY: WriteObject borrows the live compiled value in this context.
            // Preserve source/debug information, including observable error stacks.
            let buffer = unsafe {
                qjs::JS_WriteObject(
                    raw,
                    &mut length,
                    compiled.as_raw(),
                    qjs::JS_WRITE_OBJ_BYTECODE as _,
                )
            };
            if buffer.is_null() {
                return Err(Error::Exception);
            }
            // SAFETY: A successful write returns length initialized bytes allocated
            // by this context. Copy them before freeing with the same allocator.
            let bytes = unsafe { slice::from_raw_parts(buffer, length as _) }.to_vec();
            unsafe { qjs::js_free(raw, buffer.cast()) };
            // Concurrent first calls can compile independently; only one copy stays.
            let _ = self.bytecode.set(bytes);
            compiled
        };
        let _eval = profile::span("bootstrap_eval");
        // SAFETY: EvalFunction consumes its argument, so pass a duplicate of the
        // current-context owned handle. Its result is also owned by this context.
        unsafe {
            owned(
                ctx,
                qjs::JS_EvalFunction(raw, qjs::JS_DupValue(raw, compiled.as_raw())),
            )?;
        }
        Ok(())
    }
}

// SAFETY: The caller transfers an owned engine result from this exact context.
unsafe fn owned<'js>(ctx: &Ctx<'js>, raw: qjs::JSValue) -> rquickjs::Result<Value<'js>> {
    let value = unsafe { Value::from_raw(ctx.clone(), raw) };
    if value.is_exception() {
        Err(Error::Exception)
    } else {
        Ok(value)
    }
}

pub(crate) static UTILITIES: Script = Script::new(include_bytes!("../vendor/utilities.js"));
pub(crate) static SCRIPT_UTILITIES: Script =
    Script::new(include_bytes!("../vendor/script-utilities.js"));
pub(crate) static HASH: Script = Script::new(include_bytes!("../vendor/hash-utilities.js"));
pub(crate) static URL: Script = Script::new(include_bytes!("../vendor/url-utilities.js"));
pub(crate) static BOOTSTRAP: Script = Script::new(include_bytes!("script-bootstrap.js"));
pub(crate) static PRIMITIVES: Script = Script::new(include_bytes!("script-primitives.js"));
pub(crate) static WEB: Script = Script::new(include_bytes!("../vendor/web-utilities.js"));
pub(crate) static WEB_BINDINGS: Script = Script::new(include_bytes!("script-web.js"));
pub(crate) static AGE: Script = Script::new(include_bytes!("../vendor/age-utilities.js"));
pub(crate) static MMDB: Script = Script::new(include_bytes!("../vendor/mmdb-utilities.js"));
pub(crate) static DNS: Script = Script::new(include_bytes!("../vendor/dns-utilities.js"));
pub(crate) static NOTIFICATION: Script =
    Script::new(include_bytes!("../vendor/notification-utilities.js"));

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Runtime};
    use std::{sync::Arc, thread};

    #[test]
    fn bytecode_recreates_globals_and_functions_in_independent_runtimes() {
        let source =
            b"var counter = 0; let lexical = 7; function bump() { return ++counter + lexical; }";
        let script = Script::new(source);
        let inspect = "JSON.stringify([bump(),bump(),Object.getOwnPropertyDescriptor(globalThis,'counter'),bump.toString()])";
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        let expected = context.with(|ctx| {
            ctx.eval::<(), _>(source.as_slice()).unwrap();
            ctx.eval::<String, _>(inspect).unwrap()
        });
        for _ in 0..3 {
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            context.with(|ctx| {
                script.eval(&ctx).unwrap();
                assert_eq!(ctx.eval::<String, _>(inspect).unwrap(), expected);
                ctx.eval::<(), _>("counter = 1000; lexical = -1000; bump = null;")
                    .unwrap();
            });
        }
        assert!(script.bytecode.get().is_some());
    }

    #[test]
    fn bytecode_keeps_strict_errors_and_uncached_syntax_failures() {
        fn failure(ctx: &Ctx<'_>, result: rquickjs::Result<()>) -> String {
            assert!(matches!(result, Err(Error::Exception)));
            ctx.globals().set("caught", ctx.catch()).unwrap();
            ctx.eval("JSON.stringify([caught.name,caught.message,caught.stack])")
                .unwrap()
        }
        for source in [
            b"function fail() { Object.freeze({x:1}).x = 2; } fail();".as_slice(),
            b"let broken = ;".as_slice(),
        ] {
            let script = Script::new(source);
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            let expected = context.with(|ctx| failure(&ctx, ctx.eval(source)));
            for _ in 0..3 {
                let runtime = Runtime::new().unwrap();
                let context = Context::full(&runtime).unwrap();
                context.with(|ctx| {
                    assert_eq!(failure(&ctx, script.eval(&ctx)), expected);
                    assert_eq!(ctx.eval::<u32, _>("1 + 2").unwrap(), 3);
                });
            }
            assert_eq!(script.bytecode.get().is_none(), source == b"let broken = ;");
        }
    }

    #[test]
    fn bytecode_is_shared_across_threads_without_sharing_js_state() {
        let script = Arc::new(Script::new(
            b"var counter = 0; globalThis.bump = () => ++counter;",
        ));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let script = script.clone();
                thread::spawn(move || {
                    for _ in 0..4 {
                        let runtime = Runtime::new().unwrap();
                        let context = Context::full(&runtime).unwrap();
                        context.with(|ctx| {
                            script.eval(&ctx).unwrap();
                            assert!(ctx.eval::<bool, _>("bump() === 1 && bump() === 2").unwrap());
                        });
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
    }
}
