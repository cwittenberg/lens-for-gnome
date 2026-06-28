// gnome-extension/prefs_ai.js
import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import GLib from 'gi://GLib';

class AIEngineManager {
    constructor(page) {
        this.page = page;
        this._cancellables = [];
        this._timeoutIds = [];
        this.modelRows = [];
        this.switchButtons = [];
        this.buildUI();
        this.requestConfig();
        this.requestHardwareStatus();
    }

    buildUI() {
        // Persistent Hardware Status Group
        this.hwGroup = new Adw.PreferencesGroup({ title: 'Hardware Optimization' });
        this.hwStatusRow = new Adw.ActionRow({ 
             title: 'Detecting...', 
             subtitle: 'Querying backend for hardware capabilities...' 
         });
        this.hwGroup.add(this.hwStatusRow);
        this.page.add(this.hwGroup);

        // Transient Operations Group (Hidden when idle)
        this.opGroup = new Adw.PreferencesGroup({ visible: false });
        this.opStatusRow = new Adw.ActionRow({ title: 'Processing...' });
        
        this.spinner = new Gtk.Spinner({
            valign: Gtk.Align.CENTER,
            visible: false
        });
        this.opStatusRow.add_suffix(this.spinner);
        this.opGroup.add(this.opStatusRow);

        this.progressBox = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 8,
            margin_top: 4,
            margin_bottom: 12,
            margin_start: 12,
            margin_end: 12,
            visible: false
        });

        this.progressBar = new Gtk.ProgressBar({
            show_text: true,
            hexpand: true,
            valign: Gtk.Align.CENTER
        });

        this.cancelButton = new Gtk.Button({
            icon_name: 'process-stop-symbolic',
            valign: Gtk.Align.CENTER,
            tooltip_text: 'Cancel Download'
        });
        this.cancelButton.add_css_class('destructive-action');
        this.cancelButton.connect('clicked', () => this.cancelActiveOperations());

        this.progressBox.append(this.progressBar);
        this.progressBox.append(this.cancelButton);
        this.opGroup.add(this.progressBox);
        this.page.add(this.opGroup);

        // Model Selection Group
        this.modelGroup = new Adw.PreferencesGroup({ 
             title: 'Available AI Models', 
             description: 'Fetching configurations...' 
         });
        this.page.add(this.modelGroup);
    }

    _sendPayload(payloadObj, onMessage) {
        let cancellable = new Gio.Cancellable();
        this._cancellables.push(cancellable);

        let socketClient = new Gio.SocketClient();
        let socketPath = GLib.get_home_dir() + '/.local/state/gnome-lens/gnome_lens.sock';
        let address = Gio.UnixSocketAddress.new(socketPath);

        socketClient.connect_async(address, cancellable, (client, res) => {
            let connection, outputStream, inputStream;
            try {
                connection = client.connect_finish(res);
                outputStream = connection.get_output_stream();
                inputStream = new Gio.DataInputStream({ base_stream: connection.get_input_stream() });
                inputStream.set_newline_type(Gio.DataStreamNewlineType.ANY);
                
                let payloadStr = JSON.stringify(payloadObj) + '\n';
                
                outputStream.write_all_async(payloadStr, GLib.PRIORITY_DEFAULT, cancellable, (stream, writeRes) => {
                    try {
                        stream.write_all_finish(writeRes);
                        this._readLoop(inputStream, outputStream, cancellable, onMessage, connection);
                    } catch (e) {
                        this._cleanupConnection(inputStream, outputStream, connection);
                    }
                });
            } catch (e) {
                if (!cancellable.is_cancelled()) {
                    this.hwStatusRow.set_title('Service Offline');
                    this.hwStatusRow.set_subtitle('Start the background daemon in the Indexation tab to view AI settings.');
                    this.spinner.stop();
                    this.spinner.set_visible(false);
                    this.opGroup.set_visible(false);
                    this.modelGroup.set_description('Cannot fetch models while service is offline.');
                }
            }
        });
    }

    _readLoop(inputStream, outputStream, cancellable, onMessage, connection) {
        if (cancellable.is_cancelled()) {
            this._cleanupConnection(inputStream, outputStream, connection);
            return;
        }

        inputStream.read_line_async(GLib.PRIORITY_DEFAULT, cancellable, (stream, res) => {
            try {
                let lineData = stream.read_line_finish_utf8(res);
                if (lineData && lineData[0] !== null) {
                    let text = lineData[0].trim();
                    if (text.length > 0) {
                        try {
                            onMessage(JSON.parse(text));
                        } catch (err) {
                            console.warn('[Gnome Lens] Invalid JSON in stream:', text);
                        }
                    }
                    this._readLoop(inputStream, outputStream, cancellable, onMessage, connection);
                } else {
                    this._cleanupConnection(inputStream, outputStream, connection);
                }
            } catch (error) {
                this._cleanupConnection(inputStream, outputStream, connection);
            }
        });
    }

    _cleanupConnection(inputStream, outputStream, connection) {
        if (inputStream) {
            inputStream.close_async(GLib.PRIORITY_DEFAULT, null, null);
        }
        if (outputStream) {
            outputStream.close_async(GLib.PRIORITY_DEFAULT, null, null);
        }
        if (connection) {
            connection.close_async(GLib.PRIORITY_DEFAULT, null, null);
        }
    }

    cancelActiveOperations() {
        for (let c of this._cancellables) {
            c.cancel();
        }
        this._cancellables = [];
        this.spinner.stop();
        this.spinner.set_visible(false);
        this.progressBox.set_visible(false);
        
        this.opStatusRow.set_title('Operation Cancelled');
        this.opStatusRow.set_subtitle('');

        let t = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
            this.requestConfig();
            return GLib.SOURCE_REMOVE;
        });
        this._timeoutIds.push(t);
    }

    requestConfig() {
        this.opGroup.set_visible(true);
        this.spinner.set_visible(true);
        this.spinner.start();
        this.opStatusRow.set_title('Synchronizing...');
        this.opStatusRow.set_subtitle('Fetching configuration...');
        
        this._sendPayload({ action: 'get_config' }, (data) => {
            if (data.status === 'config_data') {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.opGroup.set_visible(false);
                this._renderModels(data.data);
            }
        });
    }

    requestHardwareStatus() {
        this._sendPayload({ action: 'get_hardware_status' }, (data) => {
            if (data.status === 'hardware_data') {
                let hw = data.data;
                if (hw.is_hardware_dedicated && hw.acceleration_type !== 'CPU') {
                    this.hwStatusRow.set_title(`Active: ${hw.acceleration_type} Acceleration`);
                    this.hwStatusRow.set_subtitle(`${hw.device_name} (via ${hw.api})`);
                } else {
                    this.hwStatusRow.set_title('Active: CPU Mode (Software)');
                    this.hwStatusRow.set_subtitle('No dedicated hardware acceleration detected.');
                }
            }
        });
    }

    deleteModel(modelId) {
        for (let btn of this.switchButtons) {
            btn.set_sensitive(false);
        }

        this.opGroup.set_visible(true);
        this.spinner.set_visible(true);
        this.spinner.start();
        this.opStatusRow.set_title('Deleting Model...');
        this.opStatusRow.set_subtitle('Removing model files from disk.');

        this._sendPayload({ action: 'delete_model', model_id: modelId }, (data) => {
            if (data.status === 'done') {
                this.opGroup.set_visible(false);
                this.requestConfig();
            } else if (data.status === 'error') {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.opStatusRow.set_title('Engine Error');
                this.opStatusRow.set_subtitle(data.message);
                this.requestConfig();
            }
        });
    }

    switchModel(modelId) {
        for (let btn of this.switchButtons) {
            btn.set_sensitive(false);
        }
        
        this.opGroup.set_visible(true);
        this.spinner.set_visible(true);
        this.spinner.start();
        this.progressBox.set_visible(false);
        this.progressBar.set_fraction(0.0);
        
        this.opStatusRow.set_title('Executing Model Hotswap...');
        this.opStatusRow.set_subtitle('This may take several minutes if a download is required. Do not close this window.');
        
        this._sendPayload({ action: 'update_config', key: 'active_model', value: modelId }, (data) => {
            if (data.status === 'processing') {
                this.opStatusRow.set_subtitle(data.message);
            } else if (data.status === 'downloading') {
                this.opStatusRow.set_subtitle('Downloading Model...');
                this.progressBox.set_visible(true);
                let fraction = data.progress / 100.0;
                if (fraction >= 0.0 && fraction <= 1.0) {
                    this.progressBar.set_fraction(fraction);
                }
            } else if (data.status === 'done') {
                this.opGroup.set_visible(false);
                this.progressBox.set_visible(false);
                this.requestConfig();
            } else if (data.status === 'error') {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.progressBox.set_visible(false);
                this.opStatusRow.set_title('Engine Error');
                this.opStatusRow.set_subtitle(data.message);
                this.requestConfig();
            }
        });
    }

    _renderModels(configData) {
        let activeModelId = configData.active_model;
        let models = configData.models || {};
        
        // Strip out unsupported Microsoft models from the backend list if they accidentally arrive
        for (let key in models) {
            let name = (models[key].name || '').toLowerCase();
            if (name.includes('microsoft') || key.includes('phi') || key.includes('microsoft')) {
                delete models[key];
            }
        }

        this.modelGroup.set_description('Select the local AI model to power Gnome Lens. Larger models provide better reasoning but require more system RAM. (Note: Microsoft Phi models are completely unsupported due to execution instability.)');
        
        for (let r of this.modelRows) {
            this.modelGroup.remove(r);
        }
        
        this.modelRows = [];
        this.switchButtons = [];
        for (let [id, info] of Object.entries(models)) {
            let isInstalled = info.is_installed === true;
            let installedLabel = isInstalled ? ' (Installed)' : '';
            
            let row = new Adw.ActionRow({
                title: info.name + installedLabel,
                subtitle: `${info.description}\nSize: ${info.size_gb}GB | RAM Required: ${info.ram_required_gb}GB | Params: ${info.parameters}`,
            });
            row.set_subtitle_lines(3);

            if (isInstalled && id !== activeModelId) {
                let delButton = new Gtk.Button({
                    valign: Gtk.Align.CENTER,
                    icon_name: 'user-trash-symbolic',
                    margin_end: 6,
                    tooltip_text: 'Delete Model'
                });
                delButton.add_css_class('destructive-action');
                delButton.connect('clicked', () => {
                    this.deleteModel(id);
                });
                row.add_suffix(delButton);
                this.switchButtons.push(delButton);
            }

            let button = new Gtk.Button({
                valign: Gtk.Align.CENTER,
            });

            if (id === activeModelId) {
                button.set_label('Active');
                button.set_sensitive(false);
                button.add_css_class('suggested-action');
            } else {
                button.set_label(isInstalled ? 'Switch' : 'Download & Switch');
                button.connect('clicked', () => {
                    this.switchModel(id);
                });
                this.switchButtons.push(button);
            }

            row.add_suffix(button);
            this.modelGroup.add(row);
            this.modelRows.push(row);
        }
    }

    destroy() {
        for (let c of this._cancellables) {
            c.cancel();
        }
        if (this._timeoutIds) {
            for (let t of this._timeoutIds) {
                if (t > 0) GLib.source_remove(t);
            }
            this._timeoutIds = [];
        }
    }
}

export function buildAIPage(settings, window) {
    const page = new Adw.PreferencesPage({ 
         title: 'AI Engine', 
         icon_name: 'applications-engineering-symbolic' 
     });

    const behaviorGroup = new Adw.PreferencesGroup({ title: 'Engine Behavior' });
    
    const aiFilteringRow = new Adw.SwitchRow({
        title: 'Enable AI based filtering',
        subtitle: 'Uses Rhai script generation to intelligently filter data.',
    });
    settings.bind('enable-ai-filtering', aiFilteringRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    
    behaviorGroup.add(aiFilteringRow);
    page.add(behaviorGroup);

    let aiManager = new AIEngineManager(page);

    window.connect('close-request', () => {
        aiManager.destroy();
    });

    return page;
}