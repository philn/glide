extern crate failure;
extern crate fruitbasket;
extern crate gio;
extern crate glib;

use std::boxed::Box;
use std::thread;

use cocoa::appkit::{
    NSApp, NSApplication, NSApplicationActivateIgnoringOtherApps, NSApplicationActivationPolicyRegular,
    NSBackingStoreBuffered, NSMenu, NSMenuItem, NSRunningApplication, NSView, NSWindow, NSWindowStyleMask,
};
use cocoa::base::{nil, selector, NO, YES};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use fruitbasket::ActivationPolicy;
use fruitbasket::FruitApp;
use fruitbasket::FruitError;
use fruitbasket::InstallDir;
use fruitbasket::RunPeriod;
use fruitbasket::Trampoline;
use objc::rc::StrongPtr;
use objc::runtime::Object;
use std::borrow::Borrow;

use crate::app;
use crate::channel_player::{ChannelPlayer, PlaybackState};
use crate::gobject_sys;
use crate::video_player;
use crate::video_player::GLOBAL;
use crate::video_renderer;
use crate::video_renderer_factory;
use crate::{with_mut_video_player, with_video_player};

pub struct AppData {
    //app: StrongPtr,
    app: FruitApp,
    //window: StrongPtr,
    main_loop: glib::MainLoop,
    main_context: glib::MainContext,
    file_list: Option<Vec<std::string::String>>,
}

pub struct GlideCocoaApp {
    data: Box<AppData>,
}

impl AppData {
    pub fn new() -> Self {
        let mut app = match Trampoline::new("fruitbasket", "fruitbasket", "com.trevorbentley.fruitbasket")
            .version("2.1.3")
            //.icon("fruitbasket.icns")
            .plist_key("CFBundleSpokenName", "\"fruit basket\"")
            .plist_keys(&vec![("LSMinimumSystemVersion", "10.12.0"), ("LSBackgroundOnly", "1")])
            //.resource(icon.to_str().unwrap())
            .build(InstallDir::Temp)
        {
            Err(FruitError::UnsupportedPlatform(_)) => {
                // info!("This is not a Mac.  App bundling is not supported.");
                // info!("It is still safe to use FruitApp::new(), though the dummy app will do nothing.");
                FruitApp::new()
            }
            Err(FruitError::IOError(e)) => {
                //info!("IO error! {}", e);
                std::process::exit(1);
            }
            Err(FruitError::GeneralError(e)) => {
                //info!("General error! {}", e);
                std::process::exit(1);
            }
            Ok(app) => app,
        };
        app.set_activation_policy(ActivationPolicy::Regular);

        // unsafe {
        //     let _pool = NSAutoreleasePool::new(nil);

        //     let app = NSApp();
        //     app.setActivationPolicy_(NSApplicationActivationPolicyRegular);

        //     let window = NSWindow::alloc(nil)
        //         .initWithContentRect_styleMask_backing_defer_(
        //             NSRect::new(NSPoint::new(0., 0.), NSSize::new(640., 480.)),
        //             NSWindowStyleMask::NSTitledWindowMask
        //                 | NSWindowStyleMask::NSClosableWindowMask
        //                 | NSWindowStyleMask::NSResizableWindowMask
        //                 | NSWindowStyleMask::NSMiniaturizableWindowMask,
        //             NSBackingStoreBuffered,
        //             NO,
        //         )
        //         .autorelease();
        //     window.center();

        //     let main_context = glib::MainContext::new();
        //     let main_loop = glib::MainLoop::new(Some(&main_context), false);

        //     Self {
        //         app: StrongPtr::new(app),
        //         window: StrongPtr::new(window),
        //         main_loop,
        //         main_context,
        //         file_list: None,
        //     }
        // }

        let main_context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&main_context), false);

        Self {
            app, //StrongPtr::new(app),
            //window: StrongPtr::new(window),
            main_loop,
            main_context,
            file_list: None,
        }
    }
}

impl GlideCocoaApp {
    pub fn new() -> Self {
        Self {
            data: Box::new(AppData::new()),
        }
    }
}

impl app::Application for GlideCocoaApp {
    fn set_args(&mut self, args: &Vec<std::string::String>) {
        let mut file_list = vec![];
        for file in args.iter().skip(1) {
            file_list.push(format!("file://{}", file));
        }
        self.data.file_list = Some(file_list);
    }

    fn post_init(&mut self, player: &ChannelPlayer) {
        let context = &self.data.main_context;
        dbg!(&self.data.file_list);
        eprintln!("foo 0");
        if let Some(file_list) = self.data.file_list.take() {
            //context.invoke(move || {
            eprintln!("foo 1");
            player.load_playlist(file_list.to_vec());
            //});
        }
    }
    // fn handle_cli_args(&self, ) {
    // }

    fn add_action(&self, _action: &gio::SimpleAction) {}

    fn display_about_dialog(&self) {}

    fn implementation(&self) -> Option<app::ApplicationImpl> {
        Some(app::ApplicationImpl::Cocoa(self.data.app))
        //None
    }

    fn glib_context(&self) -> Option<&glib::MainContext> {
        Some(&self.data.main_context)
    }

    fn start(&self) {
        // unsafe {
        //     self.data.window.makeKeyAndOrderFront_(nil);
        //     let current_app = NSRunningApplication::currentApplication(nil);
        //     current_app.activateWithOptions_(NSApplicationActivateIgnoringOtherApps);
        // }

        let main_loop = self.data.main_loop.clone();
        thread::spawn(move || {
            eprintln!("glib thread ID: {:?}", thread::current().id());
            main_loop.run();
        });
    }

    fn stop(&self) {}

    fn refresh_video_renderer(&self) {
        eprintln!("refresh video renderer");
    }

    fn enter_fullscreen(&self) {}
    fn leave_fullscreen(&self) {}

    fn dialog_result(&self, _relative_uri: Option<glib::GString>) -> Option<glib::GString> {
        None
    }

    fn set_video_renderer(&self, renderer: &video_renderer::VideoRenderer) {
        if let Some(implementation) = renderer.implementation() {
            match implementation {
                video_renderer::VideoWidgetImpl::Cocoa(video_window) => unsafe {
                    let window = video_window.load();
                    //self.data.window.contentView().addSubview_(*window);
                    dbg!(*window);
                },
            }
        }
    }

    fn volume_changed(&self, _volume: f64) {}

    fn set_position_range_value(&self, _position: u64) {}
    fn set_position_range_end(&self, _end: f64) {}

    fn resize_window(&self, width: i32, height: i32) {
        eprintln!("resize window to {}x{}", width, height);
    }

    fn set_window_title(&self, title: &str) {
        eprintln!("set win title to {}", title);
        unsafe {
            let title = NSString::alloc(nil).init_str(title);
            //self.data.window.setTitle_(title);
            dbg!(title);
        }
    }

    fn playback_state_changed(&self, _playback_state: &PlaybackState) {}
    fn update_subtitle_track_menu(&self, _section: &gio::Menu) {}
    fn update_audio_track_menu(&self, _section: &gio::Menu) {}
    fn update_video_track_menu(&self, _section: &gio::Menu) {}

    fn clear_audio_visualization_menu(&self) {}
    fn update_audio_visualization_menu(&self, _section: &gio::Menu) {}
    fn mutable_audio_visualization_menu(&self) -> bool {
        false
    }
}
