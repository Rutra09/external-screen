package com.virtualscreen

import android.annotation.SuppressLint
import android.graphics.BitmapFactory
import android.os.Bundle
import android.os.PowerManager
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.View
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import com.virtualscreen.databinding.ActivityMainBinding
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.*
import java.net.Socket

class MainActivity : AppCompatActivity(), SurfaceHolder.Callback {

    private lateinit var binding: ActivityMainBinding
    private lateinit var wakeLock: PowerManager.WakeLock

    private var job: Job? = null
    private var outputStream: DataOutputStream? = null
    private var displayWidth  = 1920
    private var displayHeight = 1080

    // Hardware Decoder
    private var decoder: android.media.MediaCodec? = null
    private var isDecoderConfigured = false

    // ── Prefs ─────────────────────────────────────────────────────────────────
    private val prefs get() = getSharedPreferences("vs", MODE_PRIVATE)
    private var host get() = prefs.getString("host", "192.168.1.100")!!
                set(v) { prefs.edit().putString("host", v).apply() }
    private var port get() = prefs.getInt("port", 9999)
                set(v) { prefs.edit().putInt("port", v).apply() }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)

        // Fullscreen immersive
        window.decorView.systemUiVisibility = (
            View.SYSTEM_UI_FLAG_FULLSCREEN or
            View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
            View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
        )

        // Wake lock — screen stays on while streaming
        wakeLock = (getSystemService(POWER_SERVICE) as PowerManager)
            .newWakeLock(PowerManager.SCREEN_BRIGHT_WAKE_LOCK, "VirtualScreen:wake")

        binding.surface.holder.addCallback(this)

        binding.btnSettings.setOnClickListener { showSettingsDialog() }
        binding.btnConnect.setOnClickListener {
            if (job?.isActive == true) disconnect() else connect()
        }

        setupTouchForwarding()
    }

    override fun onResume()  { super.onResume(); wakeLock.acquire(3600_000) }
    override fun onPause()   { super.onPause();  if (wakeLock.isHeld) wakeLock.release() }
    override fun onDestroy() { super.onDestroy(); disconnect() }

    // ── SurfaceHolder ─────────────────────────────────────────────────────────

    override fun surfaceCreated(holder: SurfaceHolder) {}
    override fun surfaceChanged(holder: SurfaceHolder, fmt: Int, w: Int, h: Int) {}
    override fun surfaceDestroyed(holder: SurfaceHolder) { disconnect() }

    // ── Connection ────────────────────────────────────────────────────────────

    private fun connect() {
        binding.btnConnect.text = "Disconnect"
        binding.status.text = "Connecting…"

        job = CoroutineScope(Dispatchers.IO).launch {
            try {
                Socket(host, port).use { socket ->
                    socket.tcpNoDelay = true
                    socket.soTimeout  = 5000

                    val din  = DataInputStream(BufferedInputStream(socket.getInputStream(), 256 * 1024))
                    val dout = DataOutputStream(BufferedOutputStream(socket.getOutputStream(), 4096))
                    outputStream = dout

                    // Handshake
                    val (type, payload) = readMsg(din)
                    if (type == MSG_HANDSHAKE) {
                        val hs = JSONObject(String(payload))
                        displayWidth  = hs.getInt("width")
                        displayHeight = hs.getInt("height")
                        withContext(Dispatchers.Main) {
                            binding.status.text = "Connected · ${displayWidth}×${displayHeight} (H.264)"
                            adjustSurfaceAspectRatio(displayWidth, displayHeight)
                            initDecoder(displayWidth, displayHeight)
                        }
                    }

                    socket.soTimeout = 0

                    // Frame loop
                    while (isActive) {
                        val (t, data) = readMsg(din)
                        if (t == MSG_FRAME) renderFrame(data)
                    }
                }
            } catch (e: Exception) {
                withContext(Dispatchers.Main) {
                    binding.status.text = "Disconnected: ${e.message}"
                    binding.btnConnect.text = "Connect"
                }
            }
        }
    }

    private fun disconnect() {
        job?.cancel()
        job = null
        outputStream = null
        releaseDecoder()
        runOnUiThread {
            binding.btnConnect.text = "Connect"
            binding.status.text = "Disconnected"
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    private fun initDecoder(width: Int, height: Int) {
        if (isDecoderConfigured) return
        try {
            val format = android.media.MediaFormat.createVideoFormat(android.media.MediaFormat.MIMETYPE_VIDEO_AVC, width, height)
            decoder = android.media.MediaCodec.createDecoderByType(android.media.MediaFormat.MIMETYPE_VIDEO_AVC)
            decoder?.configure(format, binding.surface.holder.surface, null, 0)
            decoder?.start()
            isDecoderConfigured = true
            
            Thread {
                val info = android.media.MediaCodec.BufferInfo()
                while (isDecoderConfigured) {
                    try {
                        val outIndex = decoder?.dequeueOutputBuffer(info, 10000) ?: -1
                        if (outIndex >= 0) {
                            decoder?.releaseOutputBuffer(outIndex, true)
                        }
                    } catch (e: Exception) {
                        e.printStackTrace()
                        break
                    }
                }
            }.start()
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    private fun releaseDecoder() {
        isDecoderConfigured = false
        try {
            decoder?.stop()
            decoder?.release()
        } catch (e: Exception) {}
        decoder = null
    }

    private fun renderFrame(h264: ByteArray) {
        val codec = decoder ?: return
        try {
            val inIndex = codec.dequeueInputBuffer(10000)
            if (inIndex >= 0) {
                val buffer = codec.getInputBuffer(inIndex)
                buffer?.clear()
                buffer?.put(h264)
                codec.queueInputBuffer(inIndex, 0, h264.size, System.nanoTime() / 1000, 0)
            }
        } catch (e: Exception) {
            e.printStackTrace()
        }
    }

    private fun adjustSurfaceAspectRatio(videoWidth: Int, videoHeight: Int) {
        val viewWidth = binding.root.width
        val viewHeight = binding.root.height
        if (viewWidth == 0 || viewHeight == 0) return
        
        val videoRatio = videoWidth.toFloat() / videoHeight.toFloat()
        val viewRatio = viewWidth.toFloat() / viewHeight.toFloat()

        val lp = binding.surface.layoutParams
        if (videoRatio > viewRatio) {
            lp.width = viewWidth
            lp.height = (viewWidth / videoRatio).toInt()
        } else {
            lp.width = (viewHeight * videoRatio).toInt()
            lp.height = viewHeight
        }
        binding.surface.layoutParams = lp
    }

    // ── Touch forwarding ──────────────────────────────────────────────────────

    @SuppressLint("ClickableViewAccessibility")
    private fun setupTouchForwarding() {
        binding.surface.setOnTouchListener { v, event ->
            val action = when (event.actionMasked) {
                MotionEvent.ACTION_DOWN       -> "down"
                MotionEvent.ACTION_UP,
                MotionEvent.ACTION_CANCEL     -> "up"
                MotionEvent.ACTION_MOVE       -> "move"
                else                          -> return@setOnTouchListener false
            }
            val nx = event.x / v.width
            val ny = event.y / v.height

            val json = JSONObject().apply {
                put("action", action)
                put("x", nx.toDouble())
                put("y", ny.toDouble())
                put("id", 0)
            }.toString().toByteArray()

            CoroutineScope(Dispatchers.IO).launch {
                try {
                    outputStream?.let { sendMsg(it, MSG_TOUCH, json) }
                } catch (_: Exception) {}
            }
            true
        }
    }

    // ── Settings dialog ───────────────────────────────────────────────────────

    private fun showSettingsDialog() {
        val view = layoutInflater.inflate(R.layout.dialog_settings, null)
        val etHost = view.findViewById<android.widget.EditText>(R.id.etHost)
        val etPort = view.findViewById<android.widget.EditText>(R.id.etPort)
        etHost.setText(host)
        etPort.setText(port.toString())

        AlertDialog.Builder(this)
            .setTitle("Server Settings")
            .setView(view)
            .setPositiveButton("Save") { _, _ ->
                host = etHost.text.toString().trim()
                port = etPort.text.toString().toIntOrNull() ?: 9999
                Toast.makeText(this, "Saved. Press Connect.", Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    // ── Protocol helpers ──────────────────────────────────────────────────────

    companion object {
        const val MSG_HANDSHAKE: Byte = 0x01
        const val MSG_FRAME: Byte     = 0x02
        const val MSG_TOUCH: Byte     = 0x03
    }

    private fun readMsg(din: DataInputStream): Pair<Byte, ByteArray> {
        val type = din.readByte()
        val len  = din.readInt()
        val buf  = ByteArray(len)
        din.readFully(buf)
        return type to buf
    }

    private fun sendMsg(dout: DataOutputStream, type: Byte, payload: ByteArray) {
        dout.writeByte(type.toInt())
        dout.writeInt(payload.size)
        dout.write(payload)
        dout.flush()
    }
}
