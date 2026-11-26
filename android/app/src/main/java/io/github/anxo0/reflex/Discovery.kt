package io.github.anxo0.reflex

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.mutableStateListOf
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.util.concurrent.ConcurrentLinkedQueue
import kotlin.concurrent.thread

/**
 * Busca PCs con Reflex en la red: por mDNS (`_reflex._tcp`) y, para redes que lo
 * bloquean, preguntando por difusión UDP.
 */
class Discovery(context: Context) {
    val pcs = mutableStateListOf<Pc>()
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val main = Handler(Looper.getMainLooper())
    private val pending = ConcurrentLinkedQueue<NsdServiceInfo>()
    @Volatile private var resolving = false
    @Volatile private var running = false
    private var listener: NsdManager.DiscoveryListener? = null

    fun start() {
        if (running) return
        running = true
        val l = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
            override fun onServiceFound(info: NsdServiceInfo) {
                pending.add(info)
                resolveNext()
            }
            override fun onServiceLost(info: NsdServiceInfo) {
                main.post { pcs.removeAll { it.name == info.serviceName } }
            }
        }
        listener = l
        runCatching { nsd.discoverServices("_reflex._tcp", NsdManager.PROTOCOL_DNS_SD, l) }
        thread(name = "reflex-udp") { while (running) { broadcast(); Thread.sleep(3000) } }
    }

    fun stop() {
        running = false
        listener?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        listener = null
    }

    @Suppress("DEPRECATION")
    private fun resolveNext() {
        if (resolving) return
        val info = pending.poll() ?: return
        resolving = true
        nsd.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                resolving = false
                resolveNext()
            }
            override fun onServiceResolved(s: NsdServiceInfo) {
                val host = s.host?.hostAddress
                val name = s.attributes["name"]?.toString(Charsets.UTF_8) ?: s.serviceName
                if (host != null && !host.contains(':')) add(Pc(name, host, s.port))
                resolving = false
                resolveNext()
            }
        })
    }

    private fun add(pc: Pc) = main.post {
        val i = pcs.indexOfFirst { it.name == pc.name || it.host == pc.host }
        if (i >= 0) pcs[i] = pc else pcs.add(pc)
    }

    /** Pregunta «REFLEX?» a toda la red y apunta quién contesta. */
    private fun broadcast() {
        runCatching {
            DatagramSocket().use { s ->
                s.broadcast = true
                s.soTimeout = 1200
                val ask = "REFLEX?".toByteArray()
                s.send(DatagramPacket(ask, ask.size, InetAddress.getByName("255.255.255.255"), Proto.PORT))
                val buf = ByteArray(256)
                while (true) {
                    val p = DatagramPacket(buf, buf.size)
                    s.receive(p)
                    val lines = String(p.data, 0, p.length).lines()
                    if (lines.size >= 3 && lines[0] == "REFLEX") {
                        val port = lines[2].trim().toIntOrNull() ?: continue
                        add(Pc(lines[1], p.address.hostAddress ?: continue, port))
                    }
                }
            }
        }
    }
}
