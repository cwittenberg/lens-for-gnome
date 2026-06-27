// gnome-extension/ui_preview_video.js
import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

let Gst = null;
let GstLoaded = false;
let GstLoadFailed = false;

async function ensureGst() {
    if (GstLoaded) return true;
    if (GstLoadFailed) return false;
    try {
        let gi = await import('gi://Gst');
        Gst = gi.default;
        Gst.init(null);
        await import('gi://GstApp');
        GstLoaded = true;
        return true;
    } catch (e) {
        console.warn(`[Gnome Lens] GStreamer unavailable. Video playback will use fallback: ${e.message}`);
        GstLoadFailed = true;
        return false;
    }
}

const GnomeLensVideoPreview = GObject.registerClass({
    GTypeName: 'GnomeLensVideoPreview'
}, class GnomeLensVideoPreview extends St.Bin {
    _init(filepath) {
        super._init({
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL
        });

        this._filepath = filepath;
        this._currentTime = 0;
        this._playbackTimerId = 0;
        this._idleRenderId = 0;
        this._pipeline = null;
        this._sink = null;
        this._busWatchId = 0;
        this._imageContent = null;
        this._contentWidth = 0;
        this._contentHeight = 0;
        this._proc = null;
        this._lastTempFile = null;
        this._emptySampleCount = 0;
        this._successfulSampleCount = 0;

        console.log(`[Gnome Lens Debug] GnomeLensVideoPreview initialized for ${filepath}`);

        this._imageActor = new Clutter.Actor({
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL,
            content_gravity: Clutter.ContentGravity.RESIZE_ASPECT
        });

        this.set_child(this._imageActor);
        this.connectObject('destroy', this._onDestroy.bind(this), this);

        this._startGstVideo();
    }

    scrub(offset) {
        console.log(`[Gnome Lens Debug] scrub called with offset: ${offset}`);
        if (this._pipeline) {
            let [success, pos] = this._pipeline.query_position(Gst.Format.TIME);
            if (success) {
                let target = pos + (offset * 1000000000);
                if (target < 0) target = 0;
                console.log(`[Gnome Lens Debug] GStreamer seeking to target nanoseconds: ${target}`);
                this._pipeline.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH | Gst.SeekFlags.KEY_UNIT, target);
            } else {
                console.log('[Gnome Lens Debug] GStreamer query_position failed during scrub.');
            }
        } else {
            this._currentTime = Math.max(0, this._currentTime + offset);
            console.log(`[Gnome Lens Debug] Fallback scrub shifting frame counter to time: ${this._currentTime}`);
            this._extractFrameAndScheduleNext(true);
        }
    }

    _onDestroy() {
        this._stopVideo();
    }

    _stopVideo() {
        console.log('[Gnome Lens Debug] _stopVideo called.');
        
        if (this._playbackTimerId > 0) {
            GLib.source_remove(this._playbackTimerId);
            this._playbackTimerId = 0;
        }

        if (this._idleRenderId > 0) {
            GLib.source_remove(this._idleRenderId);
            this._idleRenderId = 0;
        }
        
        if (this._pipeline) {
            console.log('[Gnome Lens Debug] Setting pipeline state to NULL');
            let bus = this._pipeline.get_bus();
            if (this._busWatchId > 0 && bus) {
                bus.disconnect(this._busWatchId);
                bus.remove_signal_watch();
                this._busWatchId = 0;
            }
            
            this._pipeline.set_state(Gst.State.NULL);
            
            this._pipeline = null;
            this._sink = null;
        }
        
        if (this._proc) {
            this._proc.force_exit();
            this._proc = null;
        }
        
        if (this._lastTempFile) {
            let file = Gio.File.new_for_path(this._lastTempFile);
            if (file.query_exists(null)) {
                try { file.delete(null); } catch (e) { }
            }
            this._lastTempFile = null;
        }
    }

    _fallbackToSystemThumbnail() {
        console.log('[Gnome Lens Debug] Attempting to load native system cache thumbnail fallback...');
        let uri = Gio.File.new_for_path(this._filepath).get_uri();
        let hash = GLib.compute_checksum_for_string(GLib.ChecksumType.MD5, uri, -1);
        let cacheDir = GLib.get_user_cache_dir();
        let thumbPath = GLib.build_filenamev([cacheDir, 'thumbnails', 'large', hash + '.png']);
        
        let thumbFile = Gio.File.new_for_path(thumbPath);
        if (thumbFile.query_exists(null)) {
            this.set_child(new St.Widget({
                x_expand: true, y_expand: true,
                x_align: Clutter.ActorAlign.FILL, y_align: Clutter.ActorAlign.FILL,
                style: `background-image: url("file://${thumbPath}"); background-size: contain; background-repeat: no-repeat; background-position: center;`
            }));
        }
    }
    
    async _startGstVideo() {
        console.log(`[Gnome Lens Debug] _startGstVideo initiated for: ${this._filepath}`);
        let hasGst = await ensureGst();
        
        if (!hasGst) {
            console.log('[Gnome Lens Debug] GStreamer unavailable on host system. Directing to subprocess extraction loops.');
            this._extractFrameAndScheduleNext();
            return;
        }

        this._stopVideo();

        try {
            console.log('[Gnome Lens Debug] Creating playbin...');
            let pipeline = Gst.ElementFactory.make('playbin', null);
            if (!pipeline) throw new Error("Could not construct playbin");

            pipeline.set_property('flags', 1); // 1 = GST_PLAY_FLAG_VIDEO
            pipeline.set_property('uri', Gio.File.new_for_path(this._filepath).get_uri());

            console.log('[Gnome Lens Debug] Creating appsink...');
            let sink = Gst.ElementFactory.make('appsink', null);
            if (!sink) throw new Error("Could not construct appsink");

            let caps = Gst.Caps.from_string('video/x-raw, format=RGBA');
            sink.set_property('caps', caps);
            sink.set_property('drop', true);
            sink.set_property('max-buffers', 1);
            sink.set_property('emit-signals', false); 

            pipeline.set_property('video-sink', sink);
            
            this._pipeline = pipeline;
            this._sink = sink;

            console.log('[Gnome Lens Debug] Adding bus watch...');
            let bus = pipeline.get_bus();
            bus.add_signal_watch();
            this._busWatchId = bus.connect('message', (busMsg, message) => {
                if (message.type === Gst.MessageType.STATE_CHANGED) {
                    if (message.src === pipeline) {
                        let [oldState, newState] = message.parse_state_changed();
                        console.log(`[Gnome Lens Debug] Pipeline state changed from ${oldState} to ${newState}`);
                    }
                } else if (message.type === Gst.MessageType.ASYNC_DONE) {
                    console.log('[Gnome Lens Debug] Bus Message: ASYNC_DONE (Pipeline prerolled and ready)');
                } else if (message.type === Gst.MessageType.EOS) {
                    console.log('[Gnome Lens Debug] Bus Message: End of Stream. Looping timeline back to start point.');
                    if (this._pipeline) {
                        this._pipeline.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH | Gst.SeekFlags.KEY_UNIT, 0);
                    }
                } else if (message.type === Gst.MessageType.ERROR) {
                    let [err, debug] = message.parse_error();
                    console.log(`[Gnome Lens Debug] Bus Message: ERROR - ${err.message} | ${debug}`);
                    if (this._pipeline) {
                        this._extractFrameAndScheduleNext();
                    }
                }
            });

            console.log('[Gnome Lens Debug] Setting pipeline state to PLAYING...');
            let stateReturn = pipeline.set_state(Gst.State.PLAYING);
            console.log(`[Gnome Lens Debug] Pipeline set_state(PLAYING) returned: ${stateReturn}`);

            if (stateReturn === Gst.StateChangeReturn.FAILURE) {
                throw new Error("Failed to set pipeline to PLAYING");
            }

            this._playbackTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
                if (!this._sink || !this.visible || !this._pipeline) return GLib.SOURCE_CONTINUE;
                
                let sample = null;
                try {
                    if (typeof this._sink.try_pull_sample === 'function') {
                        sample = this._sink.try_pull_sample(0);
                    } else {
                        sample = this._sink.emit('try-pull-sample', 0);
                    }
                } catch (e) {
                    console.log(`[Gnome Lens Debug] Error pulling sample from appsink: ${e.message}`);
                }
                
                if (sample) {
                    this._successfulSampleCount++;
                    if (this._successfulSampleCount <= 5) {
                        console.log(`[Gnome Lens Debug] Successfully pulled sample ${this._successfulSampleCount}`);
                    }
                    
                    if (this._idleRenderId === 0) {
                        this._idleRenderId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                            this._idleRenderId = 0;
                            if (sample && this.visible && this._pipeline) {
                                this._renderSample(sample);
                            }
                            return GLib.SOURCE_REMOVE;
                        });
                    }
                } else {
                    this._emptySampleCount++;
                    if (this._emptySampleCount <= 2) {
                        console.log(`[Gnome Lens Debug] Pulled empty sample (null) ${this._emptySampleCount}`);
                    }
                }
                
                return GLib.SOURCE_CONTINUE;
            });
            
        } catch (e) {
            console.log(`[Gnome Lens Debug] GStreamer native initialization failed: ${e.message}`);
            this._extractFrameAndScheduleNext();
        }
    }

    _renderSample(sample) {
        if (!this.visible || !this._pipeline || !sample) return;

        let caps = sample.get_caps();
        if (!caps) return;
        
        let structure = caps.get_structure(0);
        if (!structure) return;

        let width = 0;
        let height = 0;

        try {
            let [successW, w] = structure.get_int('width');
            let [successH, h] = structure.get_int('height');
            if (successW && successH) {
                width = w; height = h;
            }
        } catch (e) { }

        if (width <= 0 || height <= 0) {
            let wRes = structure.get_value('width');
            let hRes = structure.get_value('height');
            if (wRes !== null && wRes !== undefined) {
                width = typeof wRes === 'number' ? wRes : (typeof wRes.get_int === 'function' ? wRes.get_int() : parseInt(wRes));
            }
            if (hRes !== null && hRes !== undefined) {
                height = typeof hRes === 'number' ? hRes : (typeof hRes.get_int === 'function' ? hRes.get_int() : parseInt(hRes));
            }
        }
        
        if (!width || !height || width <= 0 || height <= 0) return;

        let buffer = sample.get_buffer();
        if (!buffer) return;

        let [isMapped, mapInfo] = buffer.map(Gst.MapFlags.READ);
        if (isMapped) {
            let data = mapInfo.data; 

            if (!this._imageContent || this._contentWidth !== width || this._contentHeight !== height) {
                if (typeof St.ImageContent.new_with_preferred_size === 'function') {
                    this._imageContent = St.ImageContent.new_with_preferred_size(width, height);
                } else {
                    this._imageContent = new St.ImageContent();
                }
                
                this._contentWidth = width;
                this._contentHeight = height;
                this._imageActor.set_content(this._imageContent);
            }
            
            let pixelFormat = Cogl.PixelFormat.RGBA_8888;
            let bytesSuccess = false;
            
            try {
                let glibBytes = (data instanceof GLib.Bytes) ? data : new GLib.Bytes(data);
                let coglCtx = null;
                
                try {
                    if (global.stage && global.stage.context) {
                        coglCtx = global.stage.context.get_backend().get_cogl_context();
                    }
                } catch(e) {}

                if (coglCtx) {
                    try {
                        bytesSuccess = this._imageContent.set_bytes(coglCtx, glibBytes, pixelFormat, width, height, width * 4);
                    } catch(e1) {
                        bytesSuccess = this._imageContent.set_bytes(glibBytes, pixelFormat, width, height, width * 4);
                    }
                } else {
                    bytesSuccess = this._imageContent.set_bytes(glibBytes, pixelFormat, width, height, width * 4);
                }

                if (this._successfulSampleCount === 1) {
                    console.log(`[Gnome Lens Debug] Frame Extracted: w=${width}, h=${height}, Stride=${width*4}, FormatType=${pixelFormat}`);
                }
            } catch (err) {
                console.log(`[Gnome Lens Debug] Rendering fail on sample ${this._successfulSampleCount}: ${err.message}`);
            }
            
            if (!bytesSuccess && this._successfulSampleCount === 1) {
                console.log('[Gnome Lens Debug] ALL rendering strategies failed for this frame.');
            } else if (bytesSuccess) {
                this._imageActor.queue_redraw();
            }
            
            buffer.unmap(mapInfo);
        }
    }

    _extractFrameAndScheduleNext(isScrubbing = false) {
        if (this._playbackTimerId > 0) {
            GLib.source_remove(this._playbackTimerId);
            this._playbackTimerId = 0;
        }
        
        if (this._proc) {
            this._proc.force_exit();
            this._proc = null;
        }
        
        let tempFile = GLib.build_filenamev([GLib.get_tmp_dir(), `gnome-lens-preview-${GLib.uuid_string_random()}.jpg`]);
        let cmd = ['ffmpeg', '-y', '-ss', this._currentTime.toString(), '-i', this._filepath, '-vframes', '1', '-q:v', '2', '-vf', 'scale=640:-1', tempFile];

        console.log(`[Gnome Lens Debug] Executing fallback frame extraction process command: ${cmd.join(' ')}`);

        try {
            let proc = Gio.Subprocess.new(cmd, Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            this._proc = proc;

            proc.wait_async(null, (p, res) => {
                try { proc.wait_finish(res); } catch(e) {}
                if (this._proc !== proc) return;
                this._proc = null;

                let file = Gio.File.new_for_path(tempFile);
                if (file.query_exists(null)) {
                    if (!this.visible) {
                        try { file.delete(null); } catch(e) {}
                        return;
                    }

                    this.set_child(new St.Widget({
                        x_expand: true, y_expand: true,
                        x_align: Clutter.ActorAlign.FILL, y_align: Clutter.ActorAlign.FILL,
                        style: `background-image: url("file://${tempFile}"); background-size: contain; background-repeat: no-repeat; background-position: center;`
                    }));
                    
                    if (this._lastTempFile) {
                        let lastFile = Gio.File.new_for_path(this._lastTempFile);
                        if (lastFile.query_exists(null)) {
                            try { lastFile.delete(null); } catch(e) {}
                        }
                    }
                    this._lastTempFile = tempFile;
                } else {
                    console.log(`[Gnome Lens Debug] Fallback extracted image file was not generated: ${tempFile}`);
                    if (!isScrubbing && this._currentTime > 0) {
                        this._currentTime = 0;
                        this._extractFrameAndScheduleNext();
                        return;
                    }
                }
                
                this._playbackTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
                    this._playbackTimerId = 0;
                    this._currentTime += 1;
                    this._extractFrameAndScheduleNext();
                    return GLib.SOURCE_REMOVE;
                });
            });
        } catch (e) {
            console.log(`[Gnome Lens Debug] Subprocess frame generation handler exception: ${e.message}`);
            this._proc = null;
            this._fallbackToSystemThumbnail();
        }
    }
});

export { GnomeLensVideoPreview };