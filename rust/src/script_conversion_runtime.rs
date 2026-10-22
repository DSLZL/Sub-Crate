//! Worker-local environments for synchronous, JSON-only HTTP conversion.
use super::{ConversionIntrinsics, ParserState, initialize_conversion, message, serialized};
use crate::{conversion::Error, js_language::Js, json as codec, profile};
use rquickjs::{Context, Runtime};
use serde_json::Value;
use std::{
    cell::RefCell,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

static IDLE: AtomicUsize = AtomicUsize::new(0);

fn idle_limit() -> usize {
    static LIMIT: OnceLock<usize> = OnceLock::new();
    *LIMIT.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

struct Idle;

impl Idle {
    fn reserve() -> Option<Self> {
        IDLE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            (count < idle_limit()).then_some(count + 1)
        })
        .ok()
        .map(|_| Self)
    }
}

impl Drop for Idle {
    fn drop(&mut self) {
        IDLE.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Cached {
    context: Context,
    _idle: Idle,
}

thread_local! {
    static ENVIRONMENT: RefCell<Option<Cached>> = const { RefCell::new(None) };
}

fn create() -> Result<Context, Error> {
    let setup = profile::span("runtime_setup");
    let runtime = Runtime::new().map_err(|error| Error::Legacy(error.to_string()))?;
    drop(setup);
    let setup = profile::span("context_setup");
    let context = Context::custom::<ConversionIntrinsics>(&runtime)
        .map_err(|error| Error::Legacy(error.to_string()))?;
    drop(setup);
    let _bootstrap = profile::span("bootstrap");
    context.with(|ctx| {
        initialize_conversion(&ctx).map_err(|error| Error::Legacy(message(&ctx, error)))
    })?;
    Ok(context)
}

fn with_environment(
    slot: &RefCell<Option<Cached>>,
    action: impl FnOnce(&Context) -> Result<Value, Error>,
) -> Result<Value, Error> {
    let cached = slot.borrow_mut().take();
    let context = if let Some(Cached { context, _idle }) = cached {
        let _reuse = profile::span("conversion_reuse");
        drop(_idle);
        context
    } else {
        let _miss = profile::span("conversion_miss");
        create()?
    };
    // Keep the checked-out environment on the stack so unwind cannot cache it.
    let result = action(&context);
    let _cleanup = profile::span("conversion_cleanup");
    context.with(|ctx| {
        if let Some(parser) = ctx.userdata::<ParserState>() {
            parser.0.clear_request();
        }
    });
    if result.is_ok()
        && !context.runtime().is_job_pending()
        && let Some(idle) = Idle::reserve()
    {
        let _retain = profile::span("conversion_retain");
        *slot.borrow_mut() = Some(Cached {
            context,
            _idle: idle,
        });
    } else {
        let _teardown = profile::span("runtime_teardown");
        drop(context);
    }
    result
}

fn execute(context: &Context, input: &Value) -> Result<Value, Error> {
    let _job = profile::span("job");
    context.with(|ctx| {
        let result = (|| {
            let decode = profile::span("input_decode");
            let input = ctx.json_parse(codec::stringify(input))?;
            drop(decode);
            let execute = profile::span("execute");
            let output = crate::script_http::parse_direct(&Js::new(ctx.clone()), input)?;
            drop(execute);
            let _encode = profile::span("output_encode");
            codec::parse(&serialized(&ctx, output)?)
                .map_err(|error| rquickjs::Exception::throw_message(&ctx, &error.to_string()))
        })();
        result.map_err(|error| Error::Legacy(message(&ctx, error)))
    })
}

pub(super) fn run(input: &Value) -> Result<Value, Error> {
    ENVIRONMENT.with(|slot| with_environment(slot, |context| execute(context, input)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(name: &str) -> Value {
        json!({"data":format!("http://user:pass@proxy.example:8080#{name}"),
               "client":"JSON","type":"internal"})
    }

    #[test]
    fn reuse_recovery_retention_and_worker_exit() {
        let slot = RefCell::new(None);
        for name in ["first", "second"] {
            let result = with_environment(&slot, |context| {
                context.with(|ctx| {
                    if name == "first" {
                        ctx.globals().set("reuseMarker", true).unwrap();
                    } else {
                        assert!(ctx.globals().get::<_, bool>("reuseMarker").unwrap());
                    }
                    assert!(ctx.userdata::<super::super::Job>().is_none());
                });
                execute(context, &input(name))
            })
            .unwrap();
            assert_eq!(result[0]["name"], name);
        }
        let mut bad = input("bad");
        bad["client"] = json!("not-a-target");
        assert!(with_environment(&slot, |context| execute(context, &bad)).is_err());
        assert!(slot.borrow().is_none());
        with_environment(&slot, |context| {
            context.with(|ctx| {
                assert!(
                    ctx.eval::<bool, _>("typeof reuseMarker === 'undefined'")
                        .unwrap()
                );
            });
            execute(context, &input("recovered"))
        })
        .unwrap();
        with_environment(&slot, |context| {
            context.with(|ctx| {
                ctx.eval::<(), _>("Promise.resolve().then(() => { globalThis.late = true; })")
                    .unwrap();
            });
            Ok(json!(true))
        })
        .unwrap();
        assert!(slot.borrow().is_none());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = with_environment(&slot, |_| panic!("conversion unwind"));
        }))
        .is_err());
        assert!(slot.borrow().is_none());
        assert_eq!(IDLE.load(Ordering::Relaxed), 0);

        let reservations: Vec<_> = (0..idle_limit())
            .map(|_| Idle::reserve().unwrap())
            .collect();
        with_environment(&slot, |context| execute(context, &input("overflow"))).unwrap();
        assert!(slot.borrow().is_none());
        assert!(Idle::reserve().is_none());
        drop(reservations);

        let threads = (idle_limit() + 2).min(8);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(threads + 1));
        let workers: Vec<_> = (0..threads)
            .map(|index| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    for iteration in 0..8 {
                        let name = format!("worker-{index}-{iteration}");
                        assert_eq!(run(&input(&name)).unwrap()[0]["name"], name);
                    }
                    barrier.wait();
                    barrier.wait();
                })
            })
            .collect();
        barrier.wait();
        assert!(IDLE.load(Ordering::Relaxed) <= idle_limit().min(threads));
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(IDLE.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn direct_bridge_matches_fresh_dynamic_calls_and_isolates_script_mutations() {
        let context = create().unwrap();
        let cases: Value =
            serde_json::from_str(include_str!("../../tests/performance/single-node.json")).unwrap();
        let mut inputs = Vec::new();
        for case in cases.as_array().unwrap() {
            for target in crate::conversion::ALIASES {
                inputs.push(json!({"data":case["raw"],"client":target,"type":"internal"}));
            }
        }
        for body in [
            json!({"data":false,"client":"JSON"}),
            json!({"data":null,"content":"http://a.example:80#fallback","client":null,"platform":"JSON"}),
            json!({"data":"http://a.example:80","client":"JSON","produce":null}),
            json!({"data":"http://a.example:80","client":false}),
            json!({"content":42,"platform":"missing"}),
        ] {
            inputs.push(body);
        }
        for input in inputs {
            let runtime = rquickjs::AsyncRuntime::new().unwrap();
            let oracle = rquickjs::AsyncContext::full(&runtime).await.unwrap();
            let expected = oracle
                .async_with(async |ctx| {
                    initialize_conversion(&ctx).unwrap();
                    let result = async {
                        let input = ctx.json_parse(codec::stringify(&input))?;
                        let result = crate::script_http::parse(&Js::new(ctx.clone()), input).await?;
                        serialized(&ctx, result)
                    }
                    .await;
                    result.map_err(|error| message(&ctx, error))
                })
                .await;
            let actual = execute(&context, &input)
                .map(|value| codec::stringify(&value))
                .map_err(|Error::Legacy(reason)| reason);
            assert_eq!(actual, expected, "{input}");
            context.with(|ctx| {
                if let Some(parser) = ctx.userdata::<ParserState>() {
                    parser.0.clear_request();
                }
            });
            oracle
                .async_with(async |ctx| {
                    ctx.eval::<(), _>("Object.prototype.contaminated = true; globalThis.__language.text = () => 'broken'; globalThis.ProxyUtils.parse = () => [];").unwrap();
                })
                .await;
            assert_eq!(
                execute(&context, &self::input("clean")).unwrap()[0]["name"],
                "clean"
            );
            runtime.idle().await;
        }
        context.runtime().run_gc();
        let before = context.runtime().memory_usage().memory_used_size;
        for index in 0..2048 {
            let output = execute(&context, &input(&index.to_string())).unwrap();
            assert_eq!(output[0]["name"], index.to_string());
        }
        context.runtime().run_gc();
        let after = context.runtime().memory_usage().memory_used_size;
        assert!(
            after <= before + 65536,
            "retained engine growth: {before} -> {after}"
        );
    }
}
