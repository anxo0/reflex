package io.github.anxo0.reflex

import android.net.Uri
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException
import java.net.Socket

/** Un PC con Reflex abierto en «Desde el móvil». */
data class Pc(val name: String, val host: String, val port: Int) {
    fun encode() = "$host|$port|$name"

    companion object {
        fun decode(s: String): Pc? {
            val p = s.split('|', limit = 3)
            return if (p.size == 3) Pc(p[2], p[0], p[1].toIntOrNull() ?: return null) else null
        }

        /** `reflex://IP:puerto?c=CÓDIGO&n=NOMBRE`, el QR del PC. Devuelve el PC y el código. */
        fun fromQr(text: String): Pair<Pc, String>? {
            val uri = runCatching { Uri.parse(text.trim()) }.getOrNull() ?: return null
            if (uri.scheme != "reflex") return null
            val host = uri.host ?: return null
            val port = if (uri.port > 0) uri.port else Proto.PORT
            val code = uri.getQueryParameter("c") ?: ""
            return Pc(uri.getQueryParameter("n") ?: host, host, port) to code
        }
    }
}

/** Mensajes `[tipo u8][longitud u32][datos]`. Ver docs/APP_PROTOCOL.md en el repositorio. */
object Proto {
    const val PORT = 27184

    const val HELLO = 0x01
    const val VIDEO_CONFIG = 0x02
    const val VIDEO = 0x03
    const val AUDIO = 0x04
    const val SIZE = 0x05
    const val CONTROL = 0x06
    const val BYE = 0x07

    const val ACCEPT = 0x81
    const val TOUCH = 0x82
    const val KEY = 0x83
    const val PC_BYE = 0x84
}

class Conn(val socket: Socket) {
    private val out = DataOutputStream(BufferedOutputStream(socket.getOutputStream(), 1 shl 16))
    private val inp = DataInputStream(BufferedInputStream(socket.getInputStream()))

    /** Se llama desde los hilos de vídeo, audio y control: cada mensaje sale entero. */
    @Synchronized
    fun send(type: Int, vararg parts: ByteArray) {
        out.writeByte(type)
        out.writeInt(parts.sumOf { it.size })
        for (p in parts) out.write(p)
        out.flush()
    }

    @Synchronized
    fun sendVideo(pts: Long, key: Boolean, data: ByteArray, len: Int) {
        out.writeByte(Proto.VIDEO)
        out.writeInt(9 + len)
        out.writeLong(pts)
        out.writeByte(if (key) 1 else 0)
        out.write(data, 0, len)
        out.flush()
    }

    fun read(): Pair<Int, ByteArray> {
        val type = inp.readUnsignedByte()
        val len = inp.readInt()
        if (len < 0 || len > 1 shl 20) throw IOException("mensaje no válido")
        val payload = ByteArray(len)
        inp.readFully(payload)
        return type to payload
    }

    fun close() = runCatching { socket.close() }
}
