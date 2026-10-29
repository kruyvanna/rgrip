use super::*;

// ---- connection dialog ------------------------------------------------

impl App {
    pub(super) fn dialog_ui(&mut self, ctx: &egui::Context) {
        let folders = conn_folders(&self.conns, &self.conn_folders);
        let Some(d) = &mut self.dialog else { return };
        let mut test = false;
        let mut save = false;
        let mut cancel = false;

        let modal = egui::Modal::new(Id::new("conn-dialog"))
            .frame(
                Frame::new()
                    .fill(theme::SURFACE())
                    .stroke(Stroke::new(1.0_f32, theme::BORDER()))
                    .corner_radius(12)
                    .inner_margin(Margin::same(22)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(DIALOG_WIDTH);
                ui.set_max_width(DIALOG_WIDTH);
                ui.label(
                    RichText::new(if d.is_new { "New connection" } else { "Edit connection" })
                        .heading()
                        .color(theme::TEXT()),
                );
                ui.label(RichText::new("PostgreSQL").small().color(theme::MUTED()));
                ui.add_space(14.0);

                ui.horizontal(|ui| {
                    for (tab, label) in [(DialogTab::General, "General"), (DialogTab::Ssl, "SSL"), (DialogTab::Ssh, "SSH")] {
                        let mut text = label.to_string();
                        if tab == DialogTab::Ssh && d.cfg.ssh.enabled {
                            text += " ●";
                        }
                        if segment(ui, d.tab == tab, &text) {
                            d.tab = tab;
                        }
                    }
                });
                ui.add_space(10.0);
                // Fixed-size body (fits the tallest tab) so the modal doesn't
                // jump around when switching tabs or when a test result shows.
                let body = egui::vec2(DIALOG_WIDTH, 350.0);
                ui.allocate_ui_with_layout(body, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(body);
                    match d.tab {
                        DialogTab::General => general_form(ui, d, &folders),
                        DialogTab::Ssl => ssl_form(ui, d),
                        DialogTab::Ssh => ssh_form(ui, d),
                    }
                });
                let status = egui::vec2(DIALOG_WIDTH, 44.0);
                ui.allocate_ui_with_layout(status, Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(status);
                    status_line(ui, d);
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!d.testing, Button::new("Test connection")).clicked() {
                        test = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(Button::new(RichText::new("Save").color(theme::BG())).fill(theme::ACCENT()))
                            .clicked()
                        {
                            save = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            });

        if test || save {
            match (d.port.trim().parse::<u16>(), d.ssh_port.trim().parse::<u16>()) {
                (Ok(p), Ok(sp)) => {
                    d.cfg.port = p;
                    d.cfg.ssh.port = sp;
                }
                _ => {
                    d.test = Some(Err("Ports must be numbers between 1 and 65535".into()));
                    return;
                }
            }
        }
        if test {
            d.testing = true;
            d.test = None;
            self.worker.send(Cmd::Test(d.cfg.clone()));
        }
        if save {
            let folder = d.folder.trim();
            d.cfg.folder = (!folder.is_empty()).then(|| folder.to_string());
            let cfg = d.cfg.clone();
            self.dialog = None;
            match self.conn_index(&cfg.id) {
                Some(ci) => {
                    let was_connected = !matches!(self.conns[ci].status, Status::Disconnected);
                    self.conns[ci].cfg = cfg;
                    if was_connected {
                        self.apply(Action::Disconnect(ci));
                        self.apply(Action::Connect(ci));
                    }
                }
                None => self.conns.push(Conn::new(cfg)),
            }
            self.save_config();
        } else if cancel || modal.should_close() {
            self.dialog = None;
        }
    }
}

const DIALOG_WIDTH: f32 = 400.0;

/// Test result under the form; long errors wrap within the reserved space.
fn status_line(ui: &mut Ui, d: &Dialog) {
    match &d.test {
        _ if d.testing => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(12.0));
                muted(ui, "testing…");
            });
        }
        Some(Ok(v)) => {
            ui.label(RichText::new(format!("✓  Connected · {v}")).color(theme::OK()));
        }
        Some(Err(e)) => {
            egui::ScrollArea::vertical().max_height(44.0).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(e).small().color(theme::ERROR())).wrap());
            });
        }
        None => {}
    }
}

const LABEL_WIDTH: f32 = 96.0;

const FIELD_WIDTH: f32 = DIALOG_WIDTH - LABEL_WIDTH - 14.0;

/// Horizontal text-field padding; `desired_width` excludes it.
const FIELD_PAD: i8 = 8;

fn form_grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([14.0, 9.0])
        .min_col_width(LABEL_WIDTH)
        .max_col_width(FIELD_WIDTH)
        .show(ui, add);
}

fn field(ui: &mut Ui, label: &str, edit: TextEdit<'_>) {
    ui.label(RichText::new(label).color(theme::MUTED()));
    let inner = FIELD_WIDTH - 2.0 * FIELD_PAD as f32;
    ui.add(edit.desired_width(inner).margin(Margin::symmetric(FIELD_PAD, 5)));
    ui.end_row();
}

/// Folder name, typed or picked from the folders in use.
fn folder_field(ui: &mut Ui, folder: &mut String, folders: &[String]) {
    ui.label(RichText::new("Folder").color(theme::MUTED()));
    ui.horizontal(|ui| {
        let button_w = 28.0;
        let w = FIELD_WIDTH - button_w - ui.spacing().item_spacing.x - 2.0 * FIELD_PAD as f32;
        ui.add(TextEdit::singleline(folder).hint_text(hint("None")).desired_width(w).margin(Margin::symmetric(FIELD_PAD, 5)));
        ui.add_enabled_ui(!folders.is_empty(), |ui| {
            ui.menu_button("▾", |ui| {
                for f in folders {
                    if ui.button(f).clicked() {
                        *folder = f.clone();
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Existing folders");
        });
    });
    ui.end_row();
}

/// Path field with a "…" button that opens a native file picker.
fn file_field(ui: &mut Ui, label: &str, path: &mut String, placeholder: &str) {
    ui.label(RichText::new(label).color(theme::MUTED()));
    ui.horizontal(|ui| {
        let button_w = 28.0;
        let w = FIELD_WIDTH - button_w - ui.spacing().item_spacing.x - 2.0 * FIELD_PAD as f32;
        ui.add(
            TextEdit::singleline(path)
                .hint_text(hint(placeholder))
                .desired_width(w)
                .margin(Margin::symmetric(FIELD_PAD, 5)),
        );
        if ui.add_sized([button_w, 26.0], Button::new("…")).on_hover_text("Choose file").clicked() {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = std::path::Path::new(path.as_str()).parent().filter(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(file) = dialog.pick_file() {
                *path = file.display().to_string();
            }
        }
    });
    ui.end_row();
}

fn note(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(theme::FAINT())).wrap());
}

const COLOR_HINT: &str = "Tint this connection's tabs (e.g. red for production)";

fn general_form(ui: &mut Ui, d: &mut Dialog, folders: &[String]) {
    form_grid(ui, "conn-url", |ui| {
        let edit = TextEdit::singleline(&mut d.url).hint_text(hint("postgres://user:pass@host:5432/db"));
        ui.label(RichText::new("URL").color(theme::MUTED()));
        let inner = FIELD_WIDTH - 2.0 * FIELD_PAD as f32;
        let resp = ui.add(edit.desired_width(inner).margin(Margin::symmetric(FIELD_PAD, 5)));
        ui.end_row();
        if resp.changed() {
            d.url_status = (!d.url.trim().is_empty()).then(|| {
                let mut cfg = d.cfg.clone();
                rgrip_core::conn_string::apply(&mut cfg, &d.url).map(|()| {
                    d.port = cfg.port.to_string();
                    d.cfg = cfg;
                })
            });
        }
    });
    match &d.url_status {
        Some(Ok(())) => note(ui, "Filled in from the connection string."),
        Some(Err(e)) => {
            ui.label(RichText::new(e).size(11.5).color(theme::ERROR()));
        }
        None => note(ui, "Paste a postgres:// URL, key=value string or jdbc:postgresql:// URL."),
    }
    ui.add_space(6.0);
    form_grid(ui, "conn-general", |ui| {
        let placeholder = format!("{}@{}", d.cfg.database, d.cfg.host);
        field(ui, "Name", TextEdit::singleline(&mut d.cfg.name).hint_text(hint(&placeholder)));
        folder_field(ui, &mut d.folder, folders);
        field(ui, "Host", TextEdit::singleline(&mut d.cfg.host));
        field(ui, "Port", TextEdit::singleline(&mut d.port));
        field(ui, "Database", TextEdit::singleline(&mut d.cfg.database));
        field(ui, "User", TextEdit::singleline(&mut d.cfg.user));
        field(ui, "Password", TextEdit::singleline(&mut d.cfg.password).password(true));
        ui.label(RichText::new("Color").color(theme::MUTED())).on_hover_text(COLOR_HINT);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for c in ConnColor::ALL {
                let resp = widgets::color_swatch(ui, theme::conn_color(c), d.cfg.color == c).on_hover_text(format!("{}: {COLOR_HINT}", c.label()));
                if resp.clicked() {
                    d.cfg.color = c;
                }
            }
        });
        ui.end_row();
        ui.label(RichText::new("Read-only").color(theme::MUTED()));
        ui.checkbox(&mut d.cfg.read_only, "Postgres rejects writes on this connection");
        ui.end_row();
    });
    if d.cfg.ssh.enabled {
        ui.add_space(6.0);
        note(ui, "Host and port are as seen from the SSH server.");
    }
}

fn ssl_form(ui: &mut Ui, d: &mut Dialog) {
    form_grid(ui, "conn-ssl", |ui| {
        ui.label(RichText::new("Mode").color(theme::MUTED()));
        egui::ComboBox::from_id_salt("ssl-mode")
            .selected_text(d.cfg.ssl.label())
            .width(160.0)
            .show_ui(ui, |ui| {
                for m in SslMode::ALL {
                    ui.selectable_value(&mut d.cfg.ssl, m, m.label());
                }
            });
        ui.end_row();
        file_field(ui, "CA certificate", &mut d.cfg.ssl_root_cert, "system roots");
        file_field(ui, "Client cert", &mut d.cfg.ssl_client_cert, "optional (.crt / .pem)");
        file_field(ui, "Client key", &mut d.cfg.ssl_client_key, "optional (.key / .pem)");
    });
    ui.add_space(8.0);
    note(
        ui,
        match d.cfg.ssl {
            SslMode::Disable => "No encryption.",
            SslMode::Prefer => "Encrypt if the server supports it; the certificate is not checked.",
            SslMode::Require => "Always encrypt; the certificate is not checked.",
            SslMode::VerifyCa => "Encrypt and check the certificate is signed by the CA (host name not checked).",
            SslMode::VerifyFull => "Encrypt, check the CA and that the certificate matches the host name.",
        },
    );
}

fn ssh_form(ui: &mut Ui, d: &mut Dialog) {
    let ssh = &mut d.cfg.ssh;
    ui.checkbox(&mut ssh.enabled, "Connect through an SSH tunnel");
    ui.add_space(8.0);
    ui.add_enabled_ui(ssh.enabled, |ui| {
        form_grid(ui, "conn-ssh", |ui| {
            field(ui, "SSH host", TextEdit::singleline(&mut ssh.host).hint_text(hint("bastion.example.com")));
            field(ui, "Port", TextEdit::singleline(&mut d.ssh_port));
            field(ui, "User", TextEdit::singleline(&mut ssh.user));
            ui.label(RichText::new("Auth").color(theme::MUTED()));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut ssh.auth, SshAuth::Key, "Key file");
                ui.selectable_value(&mut ssh.auth, SshAuth::Password, "Password");
            });
            ui.end_row();
            match ssh.auth {
                SshAuth::Password => {
                    field(ui, "Password", TextEdit::singleline(&mut ssh.password).password(true));
                }
                SshAuth::Key => {
                    file_field(ui, "Private key", &mut ssh.key_path, "~/.ssh/id_ed25519");
                    field(
                        ui,
                        "Passphrase",
                        TextEdit::singleline(&mut ssh.passphrase).password(true).hint_text(hint("if the key has one")),
                    );
                }
            }
        });
        ui.add_space(8.0);
        note(ui, "Host keys are checked against ~/.ssh/known_hosts; a changed key is refused.");
    });
}
