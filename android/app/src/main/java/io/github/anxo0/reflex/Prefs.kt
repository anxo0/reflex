package io.github.anxo0.reflex

import android.content.Context
import android.content.SharedPreferences
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.Locale
import java.util.UUID

/** Ajustes de la app. Son estado de Compose: la interfaz se repinta al cambiarlos. */
object Prefs {
    private lateinit var sp: SharedPreferences

    /** Calidad: 0 fluida (1280), 1 alta (1920), 2 máxima (la del móvil). */
    var quality by mutableIntStateOf(1); private set
    var fps by mutableIntStateOf(60); private set
    var audio by mutableStateOf(true); private set
    var keepScreen by mutableStateOf(true); private set
    /** 0 según el sistema, 1 claro, 2 oscuro. */
    var theme by mutableIntStateOf(0); private set
    /** 0 automático, 1 español, 2 inglés. */
    var lang by mutableIntStateOf(0); private set
    var lastPc by mutableStateOf(""); private set

    fun init(c: Context) {
        sp = c.getSharedPreferences("reflex", Context.MODE_PRIVATE)
        quality = sp.getInt("quality", 1)
        fps = sp.getInt("fps", 60)
        audio = sp.getBoolean("audio", true)
        keepScreen = sp.getBoolean("keep_screen", true)
        theme = sp.getInt("theme", 0)
        lang = sp.getInt("lang", 0)
        lastPc = sp.getString("last_pc", "") ?: ""
    }

    /** Identificador de esta instalación: el PC lo recuerda para no volver a preguntar. */
    val id: String
        get() = sp.getString("id", null) ?: UUID.randomUUID().toString().also { sp.edit().putString("id", it).apply() }

    fun saveQuality(v: Int) { quality = v; sp.edit().putInt("quality", v).apply() }
    fun saveFps(v: Int) { fps = v; sp.edit().putInt("fps", v).apply() }
    fun saveAudio(v: Boolean) { audio = v; sp.edit().putBoolean("audio", v).apply() }
    fun saveKeepScreen(v: Boolean) { keepScreen = v; sp.edit().putBoolean("keep_screen", v).apply() }
    fun saveTheme(v: Int) { theme = v; sp.edit().putInt("theme", v).apply() }
    fun saveLang(v: Int) { lang = v; sp.edit().putInt("lang", v).apply() }
    fun saveLastPc(v: String) { lastPc = v; sp.edit().putString("last_pc", v).apply() }

    /** Lado mayor del vídeo (0: el de la pantalla). */
    val maxSize: Int get() = when (quality) { 0 -> 1280; 1 -> 1920; else -> 0 }
    val bitrate: Int get() = when (quality) { 0 -> 4_000_000; 1 -> 8_000_000; else -> 16_000_000 }

    val spanish: Boolean get() = lang == 1 || (lang == 0 && Locale.getDefault().language == "es")
}

/** Texto en español o en inglés, como `t()` en la app del PC. */
fun tr(es: String, en: String): String = if (Prefs.spanish) es else en
