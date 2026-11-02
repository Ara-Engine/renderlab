use std::sync::{
    Once,
    atomic::{AtomicUsize, Ordering},
};

struct Capture {
    errors: AtomicUsize,
    missing_layer: AtomicUsize,
    instance_messages: AtomicUsize,
    active_layer: AtomicUsize,
}
static CAPTURE: Capture = Capture {
    errors: AtomicUsize::new(0),
    missing_layer: AtomicUsize::new(0),
    instance_messages: AtomicUsize::new(0),
    active_layer: AtomicUsize::new(0),
};
static INSTALL: Once = Once::new();

impl log::Log for Capture {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("wgpu_hal")
            && (metadata.level() <= log::Level::Warn
                || metadata.target() == "wgpu_hal::vulkan::instance")
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if record.level() == log::Level::Error {
            self.errors.fetch_add(1, Ordering::SeqCst);
        }
        let message = record.args().to_string();
        let active = message.contains("CURRENT-VALIDATION-ENABLED");
        let missing = message.contains("VALIDATION requested, but unable to find layer");
        if record.target() == "wgpu_hal::vulkan::instance" {
            self.instance_messages.fetch_add(1, Ordering::SeqCst);
            if missing {
                self.missing_layer.fetch_add(1, Ordering::SeqCst);
            }
            if active {
                self.active_layer.fetch_add(1, Ordering::SeqCst);
            }
        }
        if record.level() <= log::Level::Warn || active || missing {
            eprintln!(
                "gpu_diagnostic {} {} {message}",
                record.level(),
                record.target()
            );
        }
    }
    fn flush(&self) {}
}

pub struct ValidationRun {
    require_clean: bool,
    errors: usize,
    missing_layer: usize,
    instance_messages: usize,
    active_layer: usize,
}

pub fn begin(require_clean: bool) -> ValidationRun {
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).expect("validation logger must own the log sink");
        log::set_max_level(log::LevelFilter::Debug);
    });
    ValidationRun {
        require_clean,
        errors: CAPTURE.errors.load(Ordering::SeqCst),
        missing_layer: CAPTURE.missing_layer.load(Ordering::SeqCst),
        instance_messages: CAPTURE.instance_messages.load(Ordering::SeqCst),
        active_layer: CAPTURE.active_layer.load(Ordering::SeqCst),
    }
}

impl Drop for ValidationRun {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let errors = CAPTURE.errors.load(Ordering::SeqCst) - self.errors;
        let missing = CAPTURE.missing_layer.load(Ordering::SeqCst) - self.missing_layer;
        let messages = CAPTURE.instance_messages.load(Ordering::SeqCst) - self.instance_messages;
        let active = CAPTURE.active_layer.load(Ordering::SeqCst) - self.active_layer;
        eprintln!(
            "gpu_validation_summary errors={errors} missing_layer={missing} active_layer={active} instance_messages={messages} debug_build={}",
            cfg!(debug_assertions)
        );
        if !self.require_clean {
            return;
        }
        assert!(messages > 0, "Vulkan diagnostic sink was not exercised");
        assert_eq!(errors, 0, "Vulkan diagnostics reported errors");
        assert_eq!(missing, 0, "requested Vulkan validation layer unavailable");
        if cfg!(debug_assertions) {
            assert!(active > 0, "Khronos validation activation was not observed");
        }
    }
}
