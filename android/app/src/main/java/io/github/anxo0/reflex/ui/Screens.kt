package io.github.anxo0.reflex.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.anxo0.reflex.ControlService
import io.github.anxo0.reflex.MainActivity
import io.github.anxo0.reflex.Page
import io.github.anxo0.reflex.Pc
import io.github.anxo0.reflex.Prefs
import io.github.anxo0.reflex.StreamService
import io.github.anxo0.reflex.StreamState
import io.github.anxo0.reflex.tr
import kotlinx.coroutines.delay

private val cs @Composable get() = MaterialTheme.colorScheme

@Composable
fun ReflexApp(a: MainActivity) {
    Box(Modifier.fillMaxSize().background(cs.surface).systemBarsPadding()) {
        when {
            StreamState.phase != StreamState.Phase.Idle -> StreamingScreen(a)
            a.page == Page.Code -> CodeScreen(a)
            a.page == Page.Settings -> SettingsScreen(a)
            else -> HomeScreen(a)
        }
    }
}

// ---------------------------------------------------------------- piezas

@Composable
private fun TopBar(title: String, onBack: (() -> Unit)? = null, action: (@Composable () -> Unit)? = null) {
    Row(Modifier.fillMaxWidth().height(64.dp).padding(start = if (onBack == null) 20.dp else 4.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        if (onBack != null) {
            IconButton(onClick = onBack) { Icon(Icons.Back, tr("Volver", "Back"), tint = cs.onSurface) }
        }
        Text(
            title,
            fontSize = 22.sp,
            fontWeight = FontWeight.Medium,
            color = if (onBack == null) cs.primary else cs.onSurface,
            modifier = Modifier.weight(1f).padding(start = if (onBack == null) 0.dp else 4.dp),
        )
        action?.invoke()
    }
}

@Composable
private fun Group(content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.padding(horizontal = 16.dp).fillMaxWidth().clip(RoundedCornerShape(24.dp)).background(cs.surfaceContainerLowest), content = content)
}

@Composable
private fun GroupDivider(start: Int = 76) = HorizontalDivider(Modifier.padding(start = start.dp), color = cs.outlineVariant)

@Composable
private fun IconCircle(icon: ImageVector, filled: Boolean = true) {
    Box(
        Modifier.size(44.dp).clip(CircleShape).background(if (filled) cs.primaryContainer else Color.Transparent),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, tint = cs.onPrimaryContainer, modifier = Modifier.size(22.dp)) }
}

@Composable
private fun Section(text: String) {
    Text(text, fontSize = 14.sp, fontWeight = FontWeight.Medium, color = cs.primary, modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 28.dp, bottom = 8.dp))
}

@Composable
private fun Row2(title: String, subtitle: String?, icon: ImageVector?, onClick: (() -> Unit)?, trailing: @Composable () -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = if (subtitle != null) 76.dp else 60.dp)
            .let { if (onClick != null) it.clickable(onClick = onClick) else it }
            .padding(start = 16.dp, end = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (icon != null) IconCircle(icon) else Spacer(Modifier.width(4.dp))
        Column(Modifier.weight(1f).padding(vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(3.dp)) {
            Text(title, fontSize = 16.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
            if (subtitle != null) Text(subtitle, fontSize = 13.sp, color = cs.onSurfaceVariant)
        }
        trailing()
    }
}

// ---------------------------------------------------------------- 1 · Inicio

@Composable
private fun HomeScreen(a: MainActivity) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        TopBar("Reflex", action = {
            IconButton(onClick = { a.page = Page.Settings }) { Icon(Icons.Tune, tr("Ajustes", "Settings"), tint = cs.onSurfaceVariant) }
        })
        Column(Modifier.padding(start = 24.dp, end = 24.dp, top = 20.dp, bottom = 28.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(tr("¿A qué PC envías la pantalla?", "Which PC do you send your screen to?"), fontSize = 30.sp, lineHeight = 36.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
            Text(
                tr("Abre Reflex en el PC y entra en «Desde el móvil». Tenéis que estar en la misma Wi-Fi.", "Open Reflex on the PC and go to “From the phone”. Both must be on the same Wi-Fi."),
                fontSize = 15.sp, lineHeight = 22.sp, color = cs.onSurfaceVariant,
            )
        }
        Row(Modifier.padding(start = 24.dp, end = 24.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(tr("En tu red", "On your network"), fontSize = 14.sp, fontWeight = FontWeight.Medium, color = cs.primary, modifier = Modifier.weight(1f))
            Text(tr("Buscando…", "Searching…"), fontSize = 13.sp, color = cs.onSurfaceVariant)
        }
        LinearProgressIndicator(
            Modifier.padding(horizontal = 16.dp).fillMaxWidth().height(4.dp).clip(RoundedCornerShape(2.dp)),
            color = cs.secondary, trackColor = cs.surfaceVariant,
        )
        Spacer(Modifier.height(10.dp))
        val pcs = a.discovery.pcs.sortedByDescending { it.name == Prefs.lastPc }
        if (pcs.isNotEmpty()) {
            Group {
                pcs.forEachIndexed { i, pc ->
                    if (i > 0) GroupDivider()
                    Row2(
                        pc.name,
                        if (pc.name == Prefs.lastPc) tr("Usado la última vez", "Used last time") else pc.host,
                        Icons.Computer,
                        onClick = { a.connect(listOf(pc), "", "list") },
                    )
                }
            }
        }
        if (a.message.isNotEmpty()) {
            Text(a.message, fontSize = 13.sp, color = cs.error, modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 12.dp))
        }
        Section(tr("¿No aparece tu PC?", "Your PC is not listed?"))
        Group {
            Row2(tr("Escanear el QR del PC", "Scan the PC's QR"), null, Icons.Qr, onClick = { a.scanQr() })
            GroupDivider()
            Row2(tr("Escribir el código del PC", "Type the PC's code"), null, Icons.Dialpad, onClick = { a.message = ""; a.page = Page.Code })
        }
        Spacer(Modifier.height(28.dp))
        Text(
            tr("Reflex es libre y no tiene límite de tiempo. Solo se ve el móvil en el PC mientras tú lo envías.", "Reflex is free and has no time limit. Your phone only shows on the PC while you send it."),
            fontSize = 12.sp, lineHeight = 17.sp, color = cs.onSurfaceVariant, modifier = Modifier.padding(horizontal = 24.dp),
        )
    }
}

// ---------------------------------------------------------------- 2 · Conectar con código

@Composable
private fun CodeScreen(a: MainActivity) {
    var code by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Column(Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        TopBar(tr("Conectar con código", "Connect with a code"), onBack = { a.page = Page.Home })
        Column(Modifier.padding(start = 24.dp, end = 24.dp, top = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text(tr("Escribe el código que sale en el PC", "Type the code shown on the PC"), fontSize = 28.sp, lineHeight = 34.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
            Text(
                tr("Está en Reflex, en «Desde el móvil», al lado del código QR. Cambia cada vez que abres esa pantalla.", "It is in Reflex, in “From the phone”, next to the QR code. It changes every time you open that screen."),
                fontSize = 15.sp, lineHeight = 22.sp, color = cs.onSurfaceVariant,
            )
        }
        Spacer(Modifier.height(28.dp))
        BasicTextField(
            value = code,
            onValueChange = { v -> code = v.filter { it.isDigit() }.take(4) },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
            modifier = Modifier.focusRequester(focus).padding(horizontal = 24.dp),
            decorationBox = {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally)) {
                    repeat(4) { i ->
                        val active = i == code.length
                        Box(
                            Modifier.size(width = 64.dp, height = 72.dp).clip(RoundedCornerShape(16.dp)).background(cs.surfaceContainerLowest)
                                .border(if (active) 2.dp else 1.dp, if (active) cs.primary else cs.outline, RoundedCornerShape(16.dp)),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(code.getOrNull(i)?.toString() ?: "", fontSize = 32.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
                        }
                    }
                }
            },
        )
        if (a.message.isNotEmpty()) {
            Text(a.message, fontSize = 13.sp, color = cs.error, modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 16.dp))
        }
        Text(tr("o bien", "or"), fontSize = 13.sp, color = cs.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth().padding(top = 24.dp, bottom = 8.dp))
        FilledTonalButton(onClick = { a.scanQr() }, modifier = Modifier.padding(horizontal = 24.dp).fillMaxWidth().height(52.dp)) {
            Icon(Icons.Qr, null, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
            Text(tr("Escanear el QR con la cámara", "Scan the QR with the camera"), fontSize = 15.sp)
        }
        Text(
            tr("Con el código también funciona si tu PC no sale en la lista, por ejemplo en redes de empresa o de universidad.", "The code also works if your PC is not on the list, for example on work or university networks."),
            fontSize = 13.sp, lineHeight = 19.sp, color = cs.onSurfaceVariant, modifier = Modifier.padding(start = 24.dp, end = 24.dp, top = 20.dp),
        )
        Spacer(Modifier.height(24.dp))
        Button(
            onClick = { a.connectWithCode(code) },
            enabled = code.length == 4,
            modifier = Modifier.padding(horizontal = 24.dp).fillMaxWidth().height(56.dp),
            shape = RoundedCornerShape(28.dp),
        ) { Text(tr("Conectar", "Connect"), fontSize = 16.sp) }
    }
}

// ---------------------------------------------------------------- 3 · Transmitiendo

@Composable
private fun StreamingScreen(a: MainActivity) {
    val phase = StreamState.phase
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(phase) {
        while (true) {
            now = System.currentTimeMillis()
            delay(1000)
        }
    }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        TopBar(if (phase == StreamState.Phase.Streaming) tr("Enviando pantalla", "Sending screen") else "Reflex")
        when (phase) {
            StreamState.Phase.Connecting, StreamState.Phase.WaitingAccept -> {
                Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    CircularProgressIndicator(color = cs.secondary)
                    Text(
                        if (phase == StreamState.Phase.WaitingAccept) tr("Acepta en el PC", "Accept on the PC") else tr("Conectando…", "Connecting…"),
                        fontSize = 28.sp, fontWeight = FontWeight.Medium, color = cs.onSurface,
                    )
                    Text(
                        if (phase == StreamState.Phase.WaitingAccept)
                            tr("${StreamState.pcName} te preguntará si dejas que este móvil envíe su pantalla. Solo la primera vez.", "${StreamState.pcName} will ask whether to let this phone send its screen. Only the first time.")
                        else StreamState.pcName,
                        fontSize = 15.sp, lineHeight = 22.sp, color = cs.onSurfaceVariant,
                    )
                    TextButton(onClick = { StreamService.stop(a) }) { Text(tr("Cancelar", "Cancel")) }
                }
            }
            StreamState.Phase.Error -> {
                Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Text(tr("No se pudo enviar", "Could not send"), fontSize = 28.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
                    Text(StreamState.error, fontSize = 15.sp, lineHeight = 22.sp, color = cs.onSurfaceVariant)
                    Button(onClick = { StreamState.phase = StreamState.Phase.Idle }, shape = RoundedCornerShape(28.dp)) { Text(tr("Volver", "Back")) }
                }
            }
            else -> {
                val secs = ((now - StreamState.startedAt) / 1000).coerceAtLeast(0)
                val short = minOf(StreamState.width, StreamState.height)
                Column(Modifier.padding(start = 24.dp, end = 24.dp, top = 20.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(tr("Tu pantalla se ve en", "Your screen shows on"), fontSize = 15.sp, color = cs.onSurfaceVariant)
                    Text(StreamState.pcName, fontSize = 30.sp, lineHeight = 36.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
                    Text("%d:%02d · %dp · %d fps".format(secs / 60, secs % 60, short, Prefs.fps), fontSize = 14.sp, color = cs.primary, fontWeight = FontWeight.Medium)
                }
                Group {
                    Row2(tr("Sonido en el PC", "Sound on the PC"), tr("El sonido de las apps se oye en el PC", "App sound plays on the PC"), Icons.Volume, onClick = { Prefs.saveAudio(!Prefs.audio) }) {
                        Switch(checked = Prefs.audio, onCheckedChange = { Prefs.saveAudio(it) })
                    }
                    GroupDivider()
                    val control = ControlService.instance != null
                    Row2(tr("Controlar desde el PC", "Control from the PC"), tr("Con el ratón y el teclado", "With mouse and keyboard"), Icons.Mouse, onClick = if (control) null else ({ a.openAccessibility() })) {
                        if (control) Text(tr("Activado", "On"), fontSize = 14.sp, color = cs.primary, fontWeight = FontWeight.Medium)
                        else FilledTonalButton(onClick = { a.openAccessibility() }) { Text(tr("Permitir", "Allow")) }
                    }
                    GroupDivider()
                    Row2(tr("No apagar la pantalla", "Keep the screen on"), null, Icons.Sun, onClick = { Prefs.saveKeepScreen(!Prefs.keepScreen) }) {
                        Switch(checked = Prefs.keepScreen, onCheckedChange = { Prefs.saveKeepScreen(it) })
                    }
                }
                Spacer(Modifier.height(32.dp))
                Button(
                    onClick = { StreamService.stop(a) },
                    colors = ButtonDefaults.buttonColors(containerColor = cs.error, contentColor = Color.White),
                    modifier = Modifier.padding(horizontal = 24.dp).fillMaxWidth().height(56.dp),
                    shape = RoundedCornerShape(28.dp),
                ) { Text(tr("Dejar de enviar", "Stop sending"), fontSize = 16.sp) }
            }
        }
    }
}

// ---------------------------------------------------------------- 4 · Ajustes

@Composable
private fun <T> Choice(title: String, options: List<Pair<String, T>>, current: T, onPick: (T) -> Unit) {
    Column(Modifier.padding(horizontal = 20.dp, vertical = 12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(title, fontSize = 16.sp, fontWeight = FontWeight.Medium, color = cs.onSurface)
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            options.forEachIndexed { i, (label, v) ->
                SegmentedButton(selected = v == current, onClick = { onPick(v) }, shape = SegmentedButtonDefaults.itemShape(i, options.size)) {
                    Text(label, fontSize = 13.sp, maxLines = 1)
                }
            }
        }
    }
}

@Composable
private fun SettingsScreen(a: MainActivity) {
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
        TopBar(tr("Ajustes", "Settings"), onBack = { a.page = Page.Home })
        Section(tr("Transmisión", "Streaming"))
        Group {
            Choice(tr("Calidad", "Quality"), listOf(tr("Fluida", "Smooth") to 0, tr("Alta", "High") to 1, tr("Máxima", "Maximum") to 2), Prefs.quality) { Prefs.saveQuality(it) }
            GroupDivider(20)
            Choice(tr("Fluidez (fotogramas por segundo)", "Smoothness (frames per second)"), listOf("30" to 30, "60" to 60, "90" to 90), Prefs.fps) { Prefs.saveFps(it) }
            GroupDivider(20)
            Row2(tr("Enviar el sonido", "Send the sound"), tr("Android 10 o superior", "Android 10 or newer"), null, onClick = { Prefs.saveAudio(!Prefs.audio) }) {
                Switch(checked = Prefs.audio, onCheckedChange = { Prefs.saveAudio(it) })
            }
            GroupDivider(20)
            val control = ControlService.instance != null
            Row2(
                tr("Control desde el PC", "Control from the PC"),
                if (control) tr("Permitido", "Allowed") else tr("Sin permiso todavía", "Not allowed yet"),
                null,
                onClick = { a.openAccessibility() },
            ) { Icon(Icons.Chevron, null, tint = cs.onSurfaceVariant) }
        }
        Section(tr("Aplicación", "App"))
        Group {
            Choice(tr("Tema", "Theme"), listOf(tr("Sistema", "System") to 0, tr("Claro", "Light") to 1, tr("Oscuro", "Dark") to 2), Prefs.theme) { Prefs.saveTheme(it) }
            GroupDivider(20)
            Choice(tr("Idioma", "Language"), listOf(tr("Automático", "Automatic") to 0, "Español" to 1, "English" to 2), Prefs.lang) { Prefs.saveLang(it) }
        }
        Section(tr("Información", "About"))
        Group {
            Row2(tr("Ayuda", "Help"), null, null, onClick = { a.openHelp() }) { Icon(Icons.Chevron, null, tint = cs.onSurfaceVariant) }
            GroupDivider(20)
            Row2(tr("Acerca de Reflex", "About Reflex"), "${tr("Versión", "Version")} ${a.version} · GPL-3.0", null, onClick = null)
        }
    }
}
