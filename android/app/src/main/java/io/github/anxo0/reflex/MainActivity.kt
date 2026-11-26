package io.github.anxo0.reflex

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import io.github.anxo0.reflex.ui.ReflexApp
import io.github.anxo0.reflex.ui.ReflexTheme

enum class Page { Home, Code, Settings }

class MainActivity : ComponentActivity() {
    lateinit var discovery: Discovery
    var page by mutableStateOf(Page.Home)
    /** Aviso corto bajo la lista (por ejemplo, un QR que no es de Reflex). */
    var message by mutableStateOf("")

    /** A dónde se enviará cuando Android dé permiso para capturar la pantalla. */
    private var pending: Triple<List<Pc>, String, String>? = null

    private val capture = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val p = pending ?: return@registerForActivityResult
        pending = null
        if (r.resultCode == Activity.RESULT_OK && r.data != null) {
            StreamService.start(this, r.resultCode, r.data!!, p.first, p.second, p.third)
            page = Page.Home
        }
    }

    private val permissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        askCapture()
    }

    private val scan = registerForActivityResult(ScanContract()) { r ->
        r.contents?.let { openQr(it) }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Prefs.init(this)
        enableEdgeToEdge()
        discovery = Discovery(this)
        setContent {
            ReflexTheme {
                BackHandler(enabled = page != Page.Home) { page = Page.Home }
                ReflexApp(this)
            }
        }
        handleLink(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleLink(intent)
    }

    override fun onStart() {
        super.onStart()
        discovery.start()
    }

    override fun onStop() {
        discovery.stop()
        super.onStop()
    }

    /** El QR del PC escaneado con la cámara del móvil abre la app con `reflex://`. */
    private fun handleLink(i: Intent?) {
        val data = i?.data ?: return
        if (data.scheme == "reflex") openQr(data.toString())
    }

    fun openQr(text: String) {
        val (pc, code) = Pc.fromQr(text) ?: run {
            message = tr("Ese QR no es de Reflex. Escanea el que sale en el PC.", "That QR is not from Reflex. Scan the one on the PC.")
            return
        }
        connect(listOf(pc), code, "qr")
    }

    fun scanQr() {
        scan.launch(
            ScanOptions()
                .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                .setPrompt(tr("Apunta al QR de Reflex en el PC", "Point at the Reflex QR on the PC"))
                .setBeepEnabled(false)
                .setOrientationLocked(false)
        )
    }

    /** Con el código escrito se prueba en todos los PC encontrados. */
    fun connectWithCode(code: String) {
        val pcs = discovery.pcs.toList()
        if (pcs.isEmpty()) {
            message = tr("No encuentro ningún PC con Reflex en esta red. Abre «Desde el móvil» en el PC o escanea su QR.", "No PC with Reflex found on this network. Open “From the phone” on the PC or scan its QR.")
            return
        }
        connect(pcs, code, "code")
    }

    fun connect(pcs: List<Pc>, code: String, mode: String) {
        message = ""
        pending = Triple(pcs, code, mode)
        val needed = buildList {
            if (Build.VERSION.SDK_INT >= 33) add(Manifest.permission.POST_NOTIFICATIONS)
            if (Prefs.audio && Build.VERSION.SDK_INT >= 29) add(Manifest.permission.RECORD_AUDIO)
        }.filter { ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED }
        if (needed.isEmpty()) askCapture() else permissions.launch(needed.toTypedArray())
    }

    private fun askCapture() {
        val mpm = getSystemService(MediaProjectionManager::class.java)
        capture.launch(mpm.createScreenCaptureIntent())
    }

    fun openAccessibility() {
        startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    fun openHelp() {
        startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/anxo0/reflex#readme")))
    }

    val version: String
        get() = packageManager.getPackageInfo(packageName, 0).versionName ?: ""
}
