//! The connection survives independently of pane views. This root can also
//! open on a refused hello, before a session or a client exists.
mod banners;

use futures::StreamExt as _;
use gpui::{Context, Entity, Render, Subscription, Task, Window, div, prelude::*, px};
use shoal_client::{
    Event, Options,
    lifecycle::{self, SystemdOrSpawn},
    proto::Notification,
};
use shoal_config::{Config, Palette, XdgPaths};

use crate::{
    recovery::{Notices, Recovery, Status},
    session::{self, Choice, OpenError, Opened},
    settings,
    view::ShoalWindow,
};

pub struct ShoalRoot {
    content: Option<Entity<ShoalWindow>>,
    options: Options,
    choice: Choice,
    config: Config,
    palette: Palette,
    recovery: Recovery,
    notices: Notices,
    events: Option<Task<()>>,
    restart: Option<Task<()>>,
    pane_errors: Option<Subscription>,
    config_task: Option<Task<()>>,
    watcher: Option<settings::Watchers>,
}

impl ShoalRoot {
    pub fn new(
        opened: Result<Opened, OpenError>,
        options: Options,
        choice: Choice,
        config: Config,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut root = Self {
            content: None,
            options,
            choice,
            config,
            palette,
            recovery: Recovery::new(),
            notices: Notices::default(),
            events: None,
            restart: None,
            pane_errors: None,
            config_task: None,
            watcher: None,
        };
        root.install(opened, window, cx);
        root.watch_config(cx);
        root
    }

    fn install(
        &mut self,
        opened: Result<Opened, OpenError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match opened {
            Ok(opened) => {
                let Opened {
                    client,
                    hello,
                    session,
                    workspace,
                    events,
                    early,
                } = opened;
                self.choice = Choice::Session(session);
                self.recovery.connected(&self.options, &hello);
                let available = self.recovery.panes_allowed();
                let content = cx.new(|cx| {
                    ShoalWindow::new(
                        crate::view::Initial {
                            client,
                            session,
                            workspace,
                            available,
                        },
                        self.config.clone(),
                        self.palette,
                        window,
                        cx,
                    )
                });
                self.pane_errors = Some(cx.subscribe_in(
                    &content,
                    window,
                    |root, content, error: &shoal_client::Error, _, cx| {
                        root.recovery.disconnected(error);
                        content.update(cx, |view, cx| view.set_available(false, cx));
                        cx.notify();
                    },
                ));
                self.content = Some(content);
                for event in early {
                    self.on_event(event, window, cx);
                }
                let mut events = crate::forward(events);
                self.events = Some(cx.spawn_in(window, async move |root, cx| {
                    while let Some(event) = events.next().await {
                        if root
                            .update_in(cx, |root, window, cx| root.on_event(event, window, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
            Err(OpenError::Client(error)) => self.recovery.disconnected(&error),
            Err(error) => self.recovery.status = Status::Failed(error.to_string()),
        }
        cx.notify();
    }

    fn on_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        // The old client may still have queued events while stop/start runs.
        if self.recovery.status == Status::Restarting {
            return;
        }
        match &event {
            Event::Connected(hello) => self.recovery.connected(&self.options, hello),
            Event::Disconnected(error) => self.recovery.disconnected(error),
            Event::Notification(Notification::DaemonNotice { message, key, .. }) => {
                self.notices.push(key.clone(), message.clone());
            }
            _ => {}
        }
        if let Some(content) = &self.content {
            content.update(cx, |view, cx| {
                view.set_available(self.recovery.panes_allowed(), cx);
                if !view.on_event(event, window, cx) {
                    window.remove_window();
                }
            });
        }
        cx.notify();
    }

    fn restart_daemon(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.recovery.begin_restart() {
            return;
        }
        self.events = None;
        if let Some(content) = &self.content {
            content.update(cx, |view, cx| view.set_available(false, cx));
        }
        let options = self.options.clone();
        let choice = self.content.as_ref().map_or_else(
            || self.choice.clone(),
            |content| Choice::Session(content.read(cx).current_session()),
        );
        let restarting = cx.background_executor().spawn(async move {
            let connected = lifecycle::restart(options, SystemdOrSpawn::new())?;
            session::finish(connected, choice)
        });
        self.restart = Some(cx.spawn_in(window, async move |root, cx| {
            let result = restarting.await;
            root.update_in(cx, |root, window, cx| {
                root.restart = None;
                match result {
                    Ok(opened) => root.install(Ok(opened), window, cx),
                    Err(OpenError::Client(error)) => root.recovery.restart_failed(&error),
                    Err(error) => root.recovery.status = Status::Failed(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn config_update(&mut self, update: settings::Update, cx: &mut Context<Self>) {
        for (key, message) in update.notices {
            self.notices.push(Some(key), message);
        }
        self.config = update.config;
        self.palette = update.palette;
        if let Some(content) = &self.content {
            content.update(cx, |view, cx| {
                view.apply_config(&self.config, self.palette);
                cx.notify();
            });
        }
        cx.notify();
    }

    fn watch_config(&mut self, cx: &mut Context<Self>) {
        let Ok(paths) = XdgPaths::from_env() else {
            return;
        };
        match settings::watch(paths.clone()) {
            Ok((mut updates, watcher)) => {
                self.watcher = Some(watcher);
                self.config_task = Some(cx.spawn(async move |root, cx| {
                    while let Some(update) = updates.next().await {
                        if root
                            .update(cx, |root, cx| root.config_update(update, cx))
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
            Err(error) => self.notices.push(
                Some("config-watch".into()),
                format!("{}: {error}", paths.config_file().display()),
            ),
        }
    }
}

impl Render for ShoalRoot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .relative()
            .size_full()
            .text_color(self.color(self.palette.foreground));
        if let Some(content) = &self.content {
            root = root.child(content.clone());
        } else {
            root = root.bg(self.color(self.palette.background)).child(
                div()
                    .h(px(crate::chrome::TOP_BAR_HEIGHT as f32))
                    .px(px(12.0))
                    .child("Shoal"),
            );
        }
        root.children(self.overlay()).child(self.banners(cx))
    }
}
