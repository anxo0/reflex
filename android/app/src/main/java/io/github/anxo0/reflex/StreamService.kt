package io.github.anxo0.reflex

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.util.DisplayMetrics
import android.view.WindowManager
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import java.net.InetSocketAddress
import java.net.Socket
import java.nio.ByteBuffer
import kotlin.concurrent.thread

/** Lo que la interfaz enseña mientras se envía. */
object StreamState {
    enum class Phase { Idle, Connecting, WaitingAccept, Streaming, Error }

    var phase by mutableStateOf(Phase.Idle)
    var pcName by mutableStateOf("")
    var error by mutableStateOf("")
    var startedAt by mutableLongStateOf(0L)
    var width by mutableIntStateOf(0)
    var height by mutableIntStateOf(0)
}

/**
 * Captura la pantalla con MediaProjection, la codifica en H.264 y la envía al PC con
 * el sonido de las apps. Corre en primer plano mientras dura el envío.
 */
class StreamService : Service() {
    companion object {
        private const val ACTION_STOP = "stop"
        private const val CHANNEL = "stream"

        fun start(c: Context, resultCode: Int, data: Intent, candidates: List<Pc>, code: String, mode: String) {
            val i = Intent(c, StreamService::class.java)
                .putExtra("result", resultCode)
                .putExtra("data", data)
                .putStringArrayListExtra("pcs", ArrayList(candidates.map { it.encode() }))
                .putExtra("code", code)
                .putExtra("mode", mode)
            ContextCompat.startForegroundService(c, i)
        }

        fun stop(c: Context) {
            c.startService(Intent(c, StreamService::class.java).setAction(ACTION_STOP))
        }
    }

    @Volatile private var running = false
    @Volatile private var restartVideo = false
    private var projection: MediaProjection? = null
    private var display: VirtualDisplay? = null
    private var conn: Conn? = null
    private var wake: PowerManager.WakeLock? = null
    private var orientation = Configuration.ORIENTATION_UNDEFINED
    private val main = Handler(Looper.getMainLooper())

    override fun onBind(intent: Intent?): IBinder? = null

    @Suppress("DEPRECATION")
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null || intent.action == ACTION_STOP) {
            finish(null)
            return START_NOT_STICKY
        }
        if (running) return START_NOT_STICKY
        running = true
        // Android 14: el servicio tiene que estar en primer plano antes de pedir la proyección.
        val n = notification()
        if (Build.VERSION.SDK_INT >= 29) startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION) else startForeground(1, n)

        val data: Intent? = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra("data", Intent::class.java) else intent.getParcelableExtra("data")
        val mpm = getSystemService(MediaProjectionManager::class.java)
        val p = data?.let { runCatching { mpm.getMediaProjection(intent.getIntExtra("result", 0), it) }.getOrNull() }
        if (p == null) {
            finish(tr("Android no dejó capturar la pantalla", "Android did not allow screen capture"))
            return START_NOT_STICKY
        }
        projection = p
        p.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() = finish(null)
        }, main)

        val pcs = intent.getStringArrayListExtra("pcs").orEmpty().mapNotNull { Pc.decode(it) }
        val code = intent.getStringExtra("code") ?: ""
        val mode = intent.getStringExtra("mode") ?: "list"
        orientation = resources.configuration.orientation
        thread(name = "reflex-stream") { run(pcs, code, mode) }
        return START_NOT_STICKY
    }

    private fun run(pcs: List<Pc>, code: String, mode: String) {
        StreamState.error = ""
        StreamState.phase = StreamState.Phase.Connecting
        StreamState.pcName = pcs.firstOrNull()?.name ?: ""
        val (w, h) = videoSize()
        var lastError = tr("No se encontró el PC. Comprueba que Reflex está abierto en «Desde el móvil».", "The PC was not found. Check that Reflex is open in “From the phone”.")
        var connected: Conn? = null
        for (pc in pcs) {
            if (!running) return
            StreamState.pcName = pc.name
            try {
                val s = Socket()
                s.connect(InetSocketAddress(pc.host, pc.port), 4000)
                s.tcpNoDelay = true
                s.soTimeout = 65_000
                val c = Conn(s)
                val hello = buildString {
                    appendLine("v=1")
                    appendLine("name=${Build.MANUFACTURER.replaceFirstChar { it.uppercase() }} ${Build.MODEL}".let { if (Build.MODEL.startsWith(Build.MANUFACTURER, true)) "name=${Build.MODEL}" else it })
                    appendLine("id=${Prefs.id}")
                    appendLine("code=$code")
                    appendLine("mode=$mode")
                    appendLine("w=$w")
                    appendLine("h=$h")
                    appendLine("audio=${if (Prefs.audio) 1 else 0}")
                    appendLine("control=${if (ControlService.instance != null) 1 else 0}")
                }
                c.send(Proto.HELLO, hello.toByteArray())
                if (mode == "list") StreamState.phase = StreamState.Phase.WaitingAccept
                val (type, payload) = c.read()
                if (type == Proto.ACCEPT && payload.firstOrNull() == 1.toByte()) {
                    connected = c
                    Prefs.saveLastPc(pc.name)
                    break
                }
                val reason = String(payload, 1, (payload.size - 1).coerceAtLeast(0))
                c.close()
                lastError = when (reason) {
                    "code" -> tr("El código no es correcto. Mira el que sale ahora en el PC.", "The code is not right. Check the one on the PC now.")
                    else -> tr("El PC no aceptó la conexión.", "The PC did not accept the connection.")
                }
                if (reason != "code") break
            } catch (e: Exception) {
                lastError = tr("No se pudo conectar con ${pc.name}.", "Could not connect to ${pc.name}.")
            }
        }
        val c = connected ?: return finish(lastError)
        conn = c
        c.socket.soTimeout = 0
        StreamState.width = w
        StreamState.height = h
        StreamState.startedAt = System.currentTimeMillis()
        StreamState.phase = StreamState.Phase.Streaming
        main.post { updateWakeLock() }

        thread(name = "reflex-audio") { audioLoop(c) }
        thread(name = "reflex-control") { controlLoop(c) }
        try {
            videoLoop(c)
        } catch (e: Exception) {
            if (running) finish(tr("Se cortó la conexión con el PC.", "The connection to the PC was lost."))
        }
    }

    /** Tamaño del vídeo: la pantalla real reducida a la calidad elegida, en múltiplos de 16. */
    @Suppress("DEPRECATION")
    private fun videoSize(): Pair<Int, Int> {
        val wm = getSystemService(WindowManager::class.java)
        val (rw, rh) = if (Build.VERSION.SDK_INT >= 30) {
            val b = wm.maximumWindowMetrics.bounds
            b.width() to b.height()
        } else {
            val m = DisplayMetrics()
            wm.defaultDisplay.getRealMetrics(m)
            m.widthPixels to m.heightPixels
        }
        val max = Prefs.maxSize
        val scale = if (max > 0 && maxOf(rw, rh) > max) max.toFloat() / maxOf(rw, rh) else 1f
        return ((rw * scale).toInt() / 16 * 16) to ((rh * scale).toInt() / 16 * 16)
    }

    private fun videoLoop(c: Conn) {
        val info = MediaCodec.BufferInfo()
        var buf = ByteArray(1 shl 18)
        var first = true
        while (running) {
            val (w, h) = videoSize()
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                setInteger(MediaFormat.KEY_BIT_RATE, Prefs.bitrate)
                setInteger(MediaFormat.KEY_FRAME_RATE, Prefs.fps)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
                // Con la pantalla quieta el codificador no saca nada: se repite el último fotograma.
                setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            }
            val codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            val surface = codec.createInputSurface()
            codec.start()
            val dpi = resources.displayMetrics.densityDpi
            if (first) {
                display = projection?.createVirtualDisplay("Reflex", w, h, dpi, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, surface, null, null)
                first = false
            } else {
                display?.resize(w, h, dpi)
                display?.surface = surface
                c.send(Proto.SIZE, intBytes(w) + intBytes(h))
                StreamState.width = w
                StreamState.height = h
            }
            restartVideo = false
            try {
                while (running && !restartVideo) {
                    val idx = codec.dequeueOutputBuffer(info, 100_000)
                    if (idx < 0) continue
                    val out: ByteBuffer = codec.getOutputBuffer(idx) ?: continue
                    if (info.size > buf.size) buf = ByteArray(info.size * 2)
                    out.position(info.offset)
                    out.get(buf, 0, info.size)
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        c.send(Proto.VIDEO_CONFIG, buf.copyOf(info.size))
                    } else if (info.size > 0) {
                        c.sendVideo(info.presentationTimeUs, info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0, buf, info.size)
                    }
                    codec.releaseOutputBuffer(idx, false)
                }
            } finally {
                runCatching { codec.stop() }
                codec.release()
                surface.release()
            }
        }
    }

    /** El sonido de las apps (Android 10+), en PCM 48 kHz estéreo. */
    @SuppressLint("MissingPermission")
    private fun audioLoop(c: Conn) {
        if (Build.VERSION.SDK_INT < 29) return
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) return
        val p = projection ?: return
        val config = AudioPlaybackCaptureConfiguration.Builder(p)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        val format = AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(48_000).setChannelMask(AudioFormat.CHANNEL_IN_STEREO).build()
        val min = AudioRecord.getMinBufferSize(48_000, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT)
        val rec = runCatching {
            AudioRecord.Builder().setAudioFormat(format).setBufferSizeInBytes(maxOf(min, 3840 * 4)).setAudioPlaybackCaptureConfig(config).build()
        }.getOrNull() ?: return
        val chunk = ByteArray(3840) // 20 ms
        try {
            rec.startRecording()
            while (running) {
                val n = rec.read(chunk, 0, chunk.size)
                if (n > 0 && Prefs.audio) c.send(Proto.AUDIO, if (n == chunk.size) chunk else chunk.copyOf(n))
            }
        } catch (_: Exception) {
        } finally {
            runCatching { rec.stop() }
            rec.release()
        }
    }

    /** Lo que manda el PC: toques y teclas. Y avisa al PC si se puede controlar. */
    private fun controlLoop(c: Conn) {
        var lastControl = ControlService.instance != null
        thread(name = "reflex-control-state") {
            while (running) {
                val now = ControlService.instance != null
                if (now != lastControl) {
                    lastControl = now
                    runCatching { c.send(Proto.CONTROL, byteArrayOf(if (now) 1 else 0)) }
                }
                Thread.sleep(1000)
            }
        }
        try {
            while (running) {
                val (type, p) = c.read()
                when (type) {
                    Proto.TOUCH -> if (p.size >= 9) {
                        val bb = ByteBuffer.wrap(p)
                        val action = bb.get().toInt()
                        ControlService.instance?.touch(action, bb.getFloat(1), bb.getFloat(5))
                    }
                    Proto.KEY -> if (p.isNotEmpty()) ControlService.instance?.key(p[0].toInt())
                    Proto.PC_BYE -> {
                        finish(null)
                        return
                    }
                }
            }
        } catch (_: Exception) {
            if (running) finish(tr("El PC cerró la conexión.", "The PC closed the connection."))
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (newConfig.orientation != orientation) {
            orientation = newConfig.orientation
            restartVideo = true
        }
    }

    /** «No apagar la pantalla» mientras se envía. */
    @Suppress("DEPRECATION")
    fun updateWakeLock() {
        val want = running && Prefs.keepScreen
        if (want && wake == null) {
            wake = getSystemService(PowerManager::class.java).newWakeLock(PowerManager.SCREEN_BRIGHT_WAKE_LOCK, "reflex:pantalla").apply { acquire() }
        } else if (!want) {
            wake?.let { if (it.isHeld) it.release() }
            wake = null
        }
        if (running) main.postDelayed({ updateWakeLock() }, 1000)
    }

    private fun finish(error: String?) {
        val wasRunning = running
        running = false
        conn?.let { c -> thread { runCatching { c.send(Proto.BYE) }; c.close() } }
        conn = null
        main.post {
            display?.release()
            display = null
            projection?.stop()
            projection = null
            wake?.let { if (it.isHeld) it.release() }
            wake = null
            StreamState.error = error ?: ""
            StreamState.phase = if (error != null) StreamState.Phase.Error else StreamState.Phase.Idle
            if (wasRunning || error != null) {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
    }

    private fun notification(): Notification {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, getString(R.string.notif_channel), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, StreamService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(tr("Enviando la pantalla al PC", "Sending your screen to the PC"))
            .setContentText(tr("Toca para ver las opciones", "Tap for options"))
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(Notification.Action.Builder(null, tr("Dejar de enviar", "Stop sending"), stop).build())
            .build()
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }
}

private fun intBytes(v: Int) = ByteBuffer.allocate(4).putInt(v).array()
