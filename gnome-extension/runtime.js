/**
 * The backend service may run at host (when compiled via source) or from Snap
 * This module allows for abstracting both runtime environments elegantly.
 */

import GLib from 'gi://GLib';
import Gio from 'gi://Gio';

// Abstract Strategy 
class EnvironmentStrategy {
    getSocketPath() { throw new Error("Not implemented"); }
    getConfigPath(filename) { throw new Error("Not implemented"); }
    getDaemonPath() { throw new Error("Not implemented"); }
}

// Host/Native Direct Strategy (Used by run.sh)
class HostEnvironment extends EnvironmentStrategy {
    constructor(home) {
        super();
        this.home = home;
    }
    getSocketPath() { return this.home + '/.local/state/lens-for-gnome/lens_for_gnome.sock'; }
    getConfigPath(filename) { return this.home + '/.config/lens-for-gnome/' + filename; }
    getDaemonPath() {
        let paths = [
            this.home + '/.cargo/bin/lens-for-gnome',
            this.home + '/.local/bin/lens-for-gnome',
            this.home + '/Development/extensions/lens-for-gnome/target/release/lens-for-gnome',
            this.home + '/Development/extensions/lens-for-gnome/target/debug/lens-for-gnome',
            this.home + '/Projects/lens-for-gnome/target/release/lens-for-gnome',
            this.home + '/dev/lens-for-gnome/target/release/lens-for-gnome'
        ];
        for (let p of paths) {
            if (GLib.file_test(p, GLib.FileTest.EXISTS)) return p;
        }
        return GLib.find_program_in_path('lens-for-gnome') || 'lens-for-gnome.daemon';
    }
}

// Canonical Snap Strict Sandbox Strategy 
class SnapEnvironment extends EnvironmentStrategy {
    constructor(home) {
        super();
        this.home = home;
    }
    getSocketPath() { return this.home + '/snap/lens-for-gnome/current/.local/state/lens-for-gnome/lens_for_gnome.sock'; }
    getConfigPath(filename) { return this.home + '/snap/lens-for-gnome/current/.config/lens-for-gnome/' + filename; }
    getDaemonPath() { 
        if (GLib.file_test('/snap/bin/lens-for-gnome.daemon', GLib.FileTest.EXISTS)) {
            return '/snap/bin/lens-for-gnome.daemon';
        }
        return '/snap/bin/lens-for-gnome';
    }
}

// System-wide adapter
class RuntimeAdapter {
    constructor() {
        this.home = GLib.get_home_dir();
        this.hostEnv = new HostEnvironment(this.home);
        this.snapEnv = new SnapEnvironment(this.home);
        
        let inSnap = GLib.file_test(this.snapEnv.getSocketPath(), GLib.FileTest.EXISTS) ||
                     GLib.file_test('/snap/lens-for-gnome/current', GLib.FileTest.EXISTS) ||
                     GLib.file_test('/snap/bin/lens-for-gnome', GLib.FileTest.EXISTS) ||
                     GLib.file_test('/snap/bin/lens-for-gnome.daemon', GLib.FileTest.EXISTS);

        this.activeEnv = inSnap ? this.snapEnv : this.hostEnv; 
    }

    isSnap() {
        return this.activeEnv === this.snapEnv;
    }

    connectAsync(cancellable, callback) {
        let client = new Gio.SocketClient();
        let address = Gio.UnixSocketAddress.new(this.activeEnv.getSocketPath());
        
        client.connect_async(address, cancellable, (source, res) => {
            let connection = null;
            try {
                connection = client.connect_finish(res);
            } catch (e) {
                let fallbackEnv = this.activeEnv === this.snapEnv ? this.hostEnv : this.snapEnv;
                let fallbackAddress = Gio.UnixSocketAddress.new(fallbackEnv.getSocketPath());
                let fallbackClient = new Gio.SocketClient();

                fallbackClient.connect_async(fallbackAddress, cancellable, (fbSource, fbRes) => {
                    let fbConnection = null;
                    let fbError = null;
                    try {
                        fbConnection = fallbackClient.connect_finish(fbRes);
                        this.activeEnv = fallbackEnv; 
                    } catch (err) {
                        fbError = err;
                    }
                    callback(fbConnection, fbError);
                });
                return;
            }
            callback(connection, null);
        });
    }

    getConfigPath(filename) {
        return this.activeEnv.getConfigPath(filename);
    }

    getDaemonPath() {
        return this.activeEnv.getDaemonPath();
    }

    isDaemonInstalled() {
        if (GLib.file_test('/snap/lens-for-gnome/current', GLib.FileTest.EXISTS)) return true;
        if (GLib.file_test('/snap/bin/lens-for-gnome', GLib.FileTest.EXISTS)) return true;
        if (GLib.file_test('/snap/bin/lens-for-gnome.daemon', GLib.FileTest.EXISTS)) return true;
        if (GLib.find_program_in_path('lens-for-gnome')) return true;
        if (GLib.find_program_in_path('lens-for-gnome.daemon')) return true;
        return GLib.file_test(this.hostEnv.getDaemonPath(), GLib.FileTest.EXISTS);
    }

    sendPayload(payloadObj, cancellable, onMessage, onError, onOffline) {
        this.connectAsync(cancellable, (connection, error) => {
            if (error) {
                let isCancelled = error.matches ? error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED) : false;
                if (!isCancelled) {
                    if (onOffline) onOffline();
                    else if (onError) onError(error);
                }
                return;
            }

            let outputStream = connection.get_output_stream();
            let payloadStr = JSON.stringify(payloadObj) + '\n';
            
            outputStream.write_all_async(payloadStr, GLib.PRIORITY_DEFAULT, cancellable, (stream, writeRes) => {
                try {
                    stream.write_all_finish(writeRes);
                    if (onMessage) {
                        let inputStream = new Gio.DataInputStream({ base_stream: connection.get_input_stream() });
                        inputStream.set_newline_type(Gio.DataStreamNewlineType.ANY);
                        
                        let readLoop = () => {
                            inputStream.read_line_async(GLib.PRIORITY_DEFAULT, cancellable, (inStream, inRes) => {
                                try {
                                    let lineData = inStream.read_line_finish_utf8(inRes);
                                    if (lineData && lineData[0] !== null) {
                                        let text = lineData[0].trim();
                                        if (text.length > 0) {
                                            onMessage(JSON.parse(text));
                                        }
                                        readLoop();
                                    } else {
                                        this.cleanupIPC(connection, inputStream, outputStream);
                                    }
                                } catch (e) {
                                    this.cleanupIPC(connection, inputStream, outputStream);
                                }
                            });
                        };
                        readLoop();
                    } else {
                        this.cleanupIPC(connection, null, outputStream);
                    }
                } catch (e) {
                    this.cleanupIPC(connection, null, outputStream);
                    let isCancelled = e.matches ? e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED) : false;
                    if (!isCancelled && onError) onError(e);
                }
            });
        });
    }

    cleanupIPC(conn, inStream, outStream) {
        if (inStream) inStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
        if (outStream) outStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
        if (conn) conn.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
    }
}

export const runtime = new RuntimeAdapter();