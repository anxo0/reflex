; Compile with: ISCC /DVersion=0.1.0 /DOrigen=<folder with ReflexMirror.exe, LICENSE, README.md and THIRD_PARTY.md> /O<output folder> installer\reflex.iss
; Draw the screens first when the version or the design changes: installer\construir.ps1 -Version 0.1.0
;
; The wizard is replaced by one frameless Reflex window. Its background and fixed text are images
; drawn from installer\diseno; buttons, checkboxes, folder, progress and notes are pieces laid on
; top. The standard wizard still runs underneath: its buttons are moved out of sight, so Enter and
; Escape keep working.

#ifndef Version
  #error Pass the version: ISCC /DVersion=x.y.z
#endif
#ifndef Origen
  #error Pass the folder with ReflexMirror.exe, LICENSE, README.md and THIRD_PARTY.md: ISCC /DOrigen=...
#endif
#include "piezas\posiciones.iss"
#if PiezasVersion != Version
  #error The installer screens show another version: run installer\construir.ps1 -Version {#Version}
#endif

[Setup]
AppId={{2F3DBCE5-7C68-48AB-BE38-665C6BA22CE7}
AppName=Reflex
AppVersion={#Version}
AppVerName=Reflex {#Version}
AppPublisher=Reflex Project
AppPublisherURL=https://github.com/anxo0/reflex
AppSupportURL=https://github.com/anxo0/reflex/issues
AppUpdatesURL=https://github.com/anxo0/reflex/releases
AppCopyright=Copyright (C) anxo0. GPL-3.0-or-later
VersionInfoVersion={#Version}
VersionInfoCompany=Reflex Project
VersionInfoDescription=Reflex Setup
PrivilegesRequired=admin
DefaultDirName={autopf}\Reflex
DisableDirPage=yes
DisableReadyPage=yes
DisableProgramGroupPage=yes
DisableWelcomePage=no
OutputBaseFilename=Reflex-{#Version}-setup
SetupIconFile=..\assets\reflex.ico
UninstallDisplayIcon={app}\ReflexMirror.exe
UninstallDisplayName=Reflex
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern dark
WizardBackColor=#0B0E1A
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=force
RestartApplications=no
ShowLanguageDialog=auto

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
spanish.SetupWindowTitle=Instalar Reflex
spanish.ButtonNext=Instalar
spanish.ButtonFinish=Terminar
spanish.StatusExtractFiles=Copiando archivos
spanish.StatusRunProgram=Terminando la instalación
english.SetupWindowTitle=Install Reflex
english.ButtonNext=Install
english.ButtonFinish=Finish
english.StatusExtractFiles=Copying files
english.StatusRunProgram=Finishing the installation

[CustomMessages]
spanish.ElegirCarpeta=Elige dónde instalar Reflex
spanish.Salir=Reflex no está instalado todavía. ¿Quieres salir?
spanish.Firewall=Abriendo el Firewall para la conexión por Wi-Fi
spanish.Pantalla=Añadiendo Pantalla inalámbrica: puede tardar unos minutos
spanish.Reiniciar=Reinicia el PC para que «Enviar pantalla» funcione.
spanish.PantallaFallo=No se pudo añadir Pantalla inalámbrica: Reflex te la ofrecerá al usar «Enviar pantalla».
spanish.Abrir=Abrir Reflex
english.ElegirCarpeta=Choose where to install Reflex
english.Salir=Reflex is not installed yet. Do you want to leave?
english.Firewall=Opening the Firewall for Wi-Fi connections
english.Pantalla=Adding Wireless Display: it can take a few minutes
english.Reiniciar=Restart the PC so screen cast works.
english.PantallaFallo=Wireless Display could not be added: Reflex will offer it when you use screen cast.
english.Abrir=Open Reflex

[Files]
Source: "{#Origen}\ReflexMirror.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Origen}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Origen}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Origen}\THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "piezas\*.bmp"; Flags: dontcopy

[Icons]
Name: "{autoprograms}\Reflex"; Filename: "{app}\ReflexMirror.exe"
Name: "{autodesktop}\Reflex"; Filename: "{app}\ReflexMirror.exe"; Check: QuiereEscritorio

[Run]
; Wi-Fi mirroring and AirPlay listen for the phone: let them in, replacing any earlier rule.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Reflex"""; Flags: runhidden
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""Reflex"" dir=in action=allow program=""{app}\ReflexMirror.exe"" profile=private,public"; StatusMsg: "{cm:Firewall}"; Flags: runhidden
Filename: "{app}\ReflexMirror.exe"; Description: "{cm:Abrir}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
; The rule goes; the user's data in %LOCALAPPDATA%\Reflex stays.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""Reflex"""; Flags: runhidden; RunOnceId: "BorrarFirewall"

[Code]
const
  // Inno colours are BGR: the brand's dark tokens.
  ColorFondo = $1A0E0B;   // surface #0B0E1A
  ColorCampo = $2E1A15;   // surface-raised #151A2E
  ColorTinta = $FFF2EE;   // ink #EEF2FF
  ColorSuave = $C8ABA3;   // muted #A3ABC8
  ColorHielo = $FFBFA9;   // ice #A9BFFF
  AnchoVentana = 760;
  AltoVentana = 460;
  AltoCabecera = 56;
  InicioPanel = 456;
  WM_NCHITTEST = $84;
  HTCLIENT = 1;
  HTCAPTION = 2;
  GWL_WNDPROC = -4;

type
  TPieza = record
    Normal, Encima: TBitmapImage;
    X, Y, W, H: Integer;
  end;

var
  Escala, Idioma: String;
  Fondo: TBitmapImage;
  Instalar, Cancelar, CancelarInstalando, Terminar, Cambiar, Cerrar: TPieza;
  EscritorioSi, EscritorioNo, InalambricaSi, InalambricaNo, InalambricaOff, AbrirSi, AbrirNo, Relleno: TBitmapImage;
  RutaTexto, EstadoTexto, CifraTexto, AvisoTexto: TNewStaticText;
  Escritorio, Inalambrica, HayWifi, SaltarPreparando: Boolean;
  Aviso: String;
  ProcAnterior: LongInt;
  Temporizador: LongWord;

function SetWindowLong(Wnd: HWND; Index: Integer; NewLong: LongInt): LongInt;
  external 'SetWindowLongW@user32.dll stdcall';
function CallWindowProc(Proc: LongInt; Wnd: HWND; Msg: UINT; WParam, LParam: LongInt): LongInt;
  external 'CallWindowProcW@user32.dll stdcall';
function SetTimer(Wnd: HWND; Id, Elapse: LongWord; Func: LongWord): LongWord;
  external 'SetTimer@user32.dll stdcall';
function KillTimer(Wnd: HWND; Id: LongWord): Boolean;
  external 'KillTimer@user32.dll stdcall';
function GetCursorPos(var Punto: TPoint): Boolean;
  external 'GetCursorPos@user32.dll stdcall';
function ScreenToClient(Wnd: HWND; var Punto: TPoint): Boolean;
  external 'ScreenToClient@user32.dll stdcall';
function GetSystemMetrics(Index: Integer): Integer;
  external 'GetSystemMetrics@user32.dll stdcall';
function DwmSetWindowAttribute(Wnd: HWND; Atributo: Integer; var Valor: Integer; Tamano: Integer): Integer;
  external 'DwmSetWindowAttribute@dwmapi.dll stdcall';

{ Wi-Fi: "Enviar pantalla" needs an adapter whose driver can receive Miracast. }

function DetectarWifi: Boolean;
var
  Archivo, Linea, Valor: String;
  Salida: AnsiString;
  Texto: String;
  Codigo, Fin, Lineas: Integer;
begin
  Result := False;
  Archivo := ExpandConstant('{tmp}\wlan.txt');
  if not Exec(ExpandConstant('{cmd}'), '/c chcp 65001 >nul & "' + ExpandConstant('{sys}') + '\netsh.exe" wlan show drivers > "' + Archivo + '"',
    '', SW_HIDE, ewWaitUntilTerminated, Codigo) then Exit;
  if not LoadStringFromFile(Archivo, Salida) then Exit;
  Texto := LowerCase(String(Salida)) + #10;
  Lineas := 0;
  while Texto <> '' do
  begin
    Fin := Pos(#10, Texto);
    Linea := Trim(Copy(Texto, 1, Fin - 1));
    Delete(Texto, 1, Fin);
    if Pos(':', Linea) > 0 then Lineas := Lineas + 1;
    // "Monitor inalámbrico admitido: Sí (...)", "Wireless Display Supported: Yes (...)"
    if (Pos('monitor inal', Linea) = 1) or (Pos('pantalla inal', Linea) = 1) or (Pos('wireless display', Linea) = 1) then
    begin
      Valor := Trim(Copy(Linea, Pos(':', Linea) + 1, 10));
      Result := (Copy(Valor, 1, 1) = 's') or (Copy(Valor, 1, 1) = 'y');
      Exit;
    end;
  end;
  // A Windows in another language: an adapter lists its driver in many lines; none prints one or two.
  Result := Lineas > 10;
end;

function QuiereEscritorio: Boolean;
begin
  Result := Escritorio;
end;

function QuiereInalambrica: Boolean;
begin
  Result := Inalambrica and HayWifi;
end;

{ Pieces }

function Imagen(const Nombre: String; X, Y, W, H: Integer): TBitmapImage;
var
  Archivo: String;
begin
  Archivo := Nombre + '-' + Escala + '.bmp';
  ExtractTemporaryFile(Archivo);
  Result := TBitmapImage.Create(WizardForm);
  Result.Parent := WizardForm;
  Result.Left := ScaleX(X);
  Result.Top := ScaleY(Y);
  Result.Width := ScaleX(W);
  Result.Height := ScaleY(H);
  Result.Stretch := True;
  Result.Bitmap.LoadFromFile(ExpandConstant('{tmp}\') + Archivo);
end;

// Lengua is '-es', '-en', or '' for pieces without text.
function Pieza(const Nombre, Lengua: String; X, Y, W, H: Integer; AlPulsar: TNotifyEvent): TPieza;
begin
  Result.X := X; Result.Y := Y; Result.W := W; Result.H := H;
  Result.Normal := Imagen(Nombre + Lengua, X, Y, W, H);
  Result.Encima := Imagen(Nombre + '-hover' + Lengua, X, Y, W, H);
  Result.Encima.Visible := False;
  Result.Normal.Cursor := crHand;
  Result.Encima.Cursor := crHand;
  Result.Normal.OnClick := AlPulsar;
  Result.Encima.OnClick := AlPulsar;
end;

function Casilla(const Nombre: String; X, Y, W, H: Integer; AlPulsar: TNotifyEvent): TBitmapImage;
begin
  Result := Imagen(Nombre, X, Y, W, H);
  Result.Cursor := crHand;
  Result.OnClick := AlPulsar;
end;

procedure Mostrar(var P: TPieza; Visible: Boolean);
begin
  P.Normal.Visible := Visible;
  P.Encima.Visible := False;
end;

function Texto(X, Y, W, H, Tamano: Integer; Negrita: Boolean; Color, Fondo: TColor): TNewStaticText;
begin
  Result := TNewStaticText.Create(WizardForm);
  Result.Parent := WizardForm;
  Result.AutoSize := False;
  Result.Left := ScaleX(X);
  Result.Width := ScaleX(W);
  Result.Font.Name := 'Segoe UI Variable Text';
  Result.Font.Size := Tamano;
  Result.Font.Color := Color;
  if Negrita then Result.Font.Style := [fsBold];
  Result.Color := Fondo;
  Result.Caption := ' ';
  Result.AdjustHeight;
  // Centred on the line the design left for it.
  Result.Top := ScaleY(Y) + (ScaleY(H) - Result.Height) div 2;
end;

procedure Fondear(const Pantalla: String);
begin
  Fondo.Bitmap.LoadFromFile(ExpandConstant('{tmp}\fondo-') + Pantalla + '-' + Idioma + '-' + Escala + '.bmp');
end;

procedure PintarCasillas;
var
  Bienvenida, Listo: Boolean;
begin
  Bienvenida := WizardForm.CurPageID = wpWelcome;
  Listo := WizardForm.CurPageID = wpFinished;
  EscritorioSi.Visible := Bienvenida and Escritorio;
  EscritorioNo.Visible := Bienvenida and not Escritorio;
  InalambricaSi.Visible := Bienvenida and HayWifi and Inalambrica;
  InalambricaNo.Visible := Bienvenida and HayWifi and not Inalambrica;
  InalambricaOff.Visible := Bienvenida and not HayWifi;
  // The run list only fills in on the last page.
  AbrirSi.Visible := False;
  AbrirNo.Visible := False;
  if Listo and (WizardForm.RunList.Items.Count > 0) then
  begin
    AbrirSi.Visible := WizardForm.RunList.Checked[0];
    AbrirNo.Visible := not WizardForm.RunList.Checked[0];
  end;
end;

{ Hover: Inno only reports clicks, so a light timer follows the cursor. }

function Dentro(const P: TPieza; X, Y: Integer): Boolean;
begin
  Result := (X >= ScaleX(P.X)) and (X < ScaleX(P.X + P.W)) and (Y >= ScaleY(P.Y)) and (Y < ScaleY(P.Y + P.H));
end;

procedure Seguir(var P: TPieza; X, Y: Integer);
var
  Encima: Boolean;
begin
  if not (P.Normal.Visible or P.Encima.Visible) then Exit;
  Encima := Dentro(P, X, Y);
  if Encima <> P.Encima.Visible then
  begin
    P.Encima.Visible := Encima;
    P.Normal.Visible := not Encima;
  end;
end;

procedure AlTemporizador(Wnd: HWND; Msg: UINT; Id: LongWord; Tiempo: DWORD);
var
  Punto: TPoint;
begin
  if Temporizador = 0 then Exit;
  // Closing Reflex shows a page of its own: the user already chose to install, so carry on.
  if SaltarPreparando and (WizardForm.CurPageID = wpPreparing) then
  begin
    SaltarPreparando := False;
    WizardForm.NextButton.OnClick(WizardForm.NextButton);
    Exit;
  end;
  if not GetCursorPos(Punto) then Exit;
  ScreenToClient(WizardForm.Handle, Punto);
  Seguir(Instalar, Punto.X, Punto.Y);
  Seguir(Cancelar, Punto.X, Punto.Y);
  Seguir(CancelarInstalando, Punto.X, Punto.Y);
  Seguir(Terminar, Punto.X, Punto.Y);
  Seguir(Cambiar, Punto.X, Punto.Y);
  Seguir(Cerrar, Punto.X, Punto.Y);
  // The Firewall step reports itself through the hidden status label.
  if EstadoTexto.Visible and (WizardForm.StatusLabel.Caption <> '') and (EstadoTexto.Caption <> WizardForm.StatusLabel.Caption) then
    EstadoTexto.Caption := WizardForm.StatusLabel.Caption;
end;

{ Dragging: the top band and the blue panel behave as the title bar. }

function ProcVentana(Wnd: HWND; Msg: UINT; WParam, LParam: LongInt): LongInt;
var
  Punto: TPoint;
begin
  Result := CallWindowProc(ProcAnterior, Wnd, Msg, WParam, LParam);
  if (Msg = WM_NCHITTEST) and (Result = HTCLIENT) then
  begin
    Punto.X := LParam and $FFFF;
    Punto.Y := (LParam shr 16) and $FFFF;
    if Punto.X > $7FFF then Punto.X := Punto.X - $10000;
    if Punto.Y > $7FFF then Punto.Y := Punto.Y - $10000;
    ScreenToClient(Wnd, Punto);
    if ((Punto.Y < ScaleY(AltoCabecera)) or (Punto.X >= ScaleX(InicioPanel))) and not Dentro(Cerrar, Punto.X, Punto.Y) then
      Result := HTCAPTION;
  end;
end;

{ Hooks must be released before the window goes away: Windows would still call them. }

procedure SoltarArrastre;
begin
  if ProcAnterior <> 0 then
  begin
    SetWindowLong(WizardForm.Handle, GWL_WNDPROC, ProcAnterior);
    ProcAnterior := 0;
  end;
end;

procedure Soltar;
begin
  if Temporizador <> 0 then
  begin
    KillTimer(0, Temporizador);
    Temporizador := 0;
  end;
  SoltarArrastre;
end;

{ Actions }

procedure Siguiente;
begin
  WizardForm.NextButton.OnClick(WizardForm.NextButton);
end;

procedure PulsarInstalar(Sender: TObject);
begin
  if WizardForm.CurPageID = wpFinished then Soltar;
  Siguiente;
end;

procedure PulsarCancelar(Sender: TObject);
begin
  if WizardForm.CurPageID = wpFinished then PulsarInstalar(Sender)
  else WizardForm.CancelButton.OnClick(WizardForm.CancelButton);
end;

procedure PulsarCambiar(Sender: TObject);
var
  Carpeta: String;
begin
  Carpeta := WizardForm.DirEdit.Text;
  if BrowseForFolder(CustomMessage('ElegirCarpeta'), Carpeta, True) then
  begin
    WizardForm.DirEdit.Text := Carpeta;
    RutaTexto.Caption := MinimizePathName(Carpeta, RutaTexto.Font, RutaTexto.Width);
  end;
end;

procedure PulsarEscritorio(Sender: TObject);
begin
  Escritorio := not Escritorio;
  PintarCasillas;
end;

procedure PulsarInalambrica(Sender: TObject);
begin
  Inalambrica := not Inalambrica;
  PintarCasillas;
end;

procedure PulsarAbrir(Sender: TObject);
begin
  WizardForm.RunList.Checked[0] := not WizardForm.RunList.Checked[0];
  PintarCasillas;
end;

{ Wizard }

procedure EsconderAsistente;
begin
  // Out of sight but alive: they still drive the pages and answer Enter and Escape.
  WizardForm.NextButton.Left := -ScaleX(2000);
  WizardForm.CancelButton.Left := -ScaleX(2000);
  WizardForm.BackButton.Left := -ScaleX(2000);
  WizardForm.OuterNotebook.Visible := False;
  WizardForm.Bevel.Visible := False;
end;

procedure InitializeWizard;
var
  Redondo: Integer;
begin
  Escritorio := True;
  Inalambrica := False;
  SaltarPreparando := True;
  HayWifi := DetectarWifi;
  Log('Wi-Fi able to receive screen cast: ' + IntToStr(Ord(HayWifi)));
  // A silent install shows no window: nothing to draw.
  if WizardSilent then Exit;
  if ScaleX(100) >= 175 then Escala := '200'
  else if ScaleX(100) >= 138 then Escala := '150'
  else if ScaleX(100) >= 113 then Escala := '125'
  else Escala := '100';
  if ActiveLanguage = 'spanish' then Idioma := 'es' else Idioma := 'en';

  WizardForm.BorderStyle := bsNone;
  WizardForm.ClientWidth := ScaleX(AnchoVentana);
  WizardForm.ClientHeight := ScaleY(AltoVentana);
  WizardForm.Left := (GetSystemMetrics(0) - WizardForm.Width) div 2;
  WizardForm.Top := (GetSystemMetrics(1) - WizardForm.Height) div 2;
  EsconderAsistente;

  ExtractTemporaryFile('fondo-instalando-' + Idioma + '-' + Escala + '.bmp');
  ExtractTemporaryFile('fondo-listo-' + Idioma + '-' + Escala + '.bmp');
  Fondo := Imagen('fondo-bienvenida-' + Idioma, 0, 0, AnchoVentana, AltoVentana);

  Instalar := Pieza('instalar', '-' + Idioma, {#instalar_X}, {#instalar_Y}, {#instalar_W}, {#instalar_H}, @PulsarInstalar);
  Cancelar := Pieza('cancelar', '-' + Idioma, {#cancelar_X}, {#cancelar_Y}, {#cancelar_W}, {#cancelar_H}, @PulsarCancelar);
  CancelarInstalando := Pieza('cancelar', '-' + Idioma, {#cancelar_instalando_X}, {#cancelar_instalando_Y}, {#cancelar_instalando_W}, {#cancelar_instalando_H}, @PulsarCancelar);
  Terminar := Pieza('terminar', '-' + Idioma, {#terminar_X}, {#terminar_Y}, {#terminar_W}, {#terminar_H}, @PulsarInstalar);
  Cambiar := Pieza('cambiar', '-' + Idioma, {#cambiar_X}, {#cambiar_Y}, {#cambiar_W}, {#cambiar_H}, @PulsarCambiar);
  Cerrar := Pieza('cerrar', '', {#cerrar_X}, {#cerrar_Y}, {#cerrar_W}, {#cerrar_H}, @PulsarCancelar);

  EscritorioSi := Casilla('escritorio-' + Idioma, {#escritorio_X}, {#escritorio_Y}, {#escritorio_W}, {#escritorio_H}, @PulsarEscritorio);
  EscritorioNo := Casilla('escritorio-no-' + Idioma, {#escritorio_X}, {#escritorio_Y}, {#escritorio_W}, {#escritorio_H}, @PulsarEscritorio);
  InalambricaSi := Casilla('inalambrica-' + Idioma, {#inalambrica_X}, {#inalambrica_Y}, {#inalambrica_W}, {#inalambrica_H}, @PulsarInalambrica);
  InalambricaNo := Casilla('inalambrica-no-' + Idioma, {#inalambrica_X}, {#inalambrica_Y}, {#inalambrica_W}, {#inalambrica_H}, @PulsarInalambrica);
  InalambricaOff := Imagen('inalambrica-off-' + Idioma, {#inalambrica_X}, {#inalambrica_Y}, {#inalambrica_W}, {#inalambrica_H});
  AbrirSi := Casilla('abrir-' + Idioma, {#abrir_X}, {#abrir_Y}, {#abrir_W}, {#abrir_H}, @PulsarAbrir);
  AbrirNo := Casilla('abrir-no-' + Idioma, {#abrir_X}, {#abrir_Y}, {#abrir_W}, {#abrir_H}, @PulsarAbrir);

  RutaTexto := Texto({#ruta_X}, {#ruta_Y}, {#ruta_W}, {#ruta_H}, 10, False, ColorTinta, ColorCampo);
  RutaTexto.Caption := MinimizePathName(WizardForm.DirEdit.Text, RutaTexto.Font, RutaTexto.Width);
  EstadoTexto := Texto({#estado_X}, {#estado_Y}, {#cifra_X} - {#estado_X} - 8, {#estado_H}, 10, False, ColorSuave, ColorFondo);
  CifraTexto := Texto({#cifra_X}, {#cifra_Y}, {#cifra_W}, {#cifra_H}, 10, True, ColorTinta, ColorFondo);
  CifraTexto.Alignment := taRightJustify;
  Relleno := Imagen('relleno', {#relleno_X}, {#relleno_Y}, {#relleno_W}, {#relleno_H});
  Relleno.Stretch := False;
  AvisoTexto := Texto({#aviso_X}, {#aviso_Y}, {#aviso_W}, {#aviso_H}, 10, False, ColorHielo, ColorFondo);

  // Windows 11 rounds the corners of the frameless window.
  Redondo := 2;
  DwmSetWindowAttribute(WizardForm.Handle, 33, Redondo, 4);
  ProcAnterior := SetWindowLong(WizardForm.Handle, GWL_WNDPROC, CreateCallback(@ProcVentana));
  Temporizador := SetTimer(0, 0, 40, CreateCallback(@AlTemporizador));
end;

procedure CurPageChanged(CurPageID: Integer);
var
  Bienvenida, Instalando, Listo: Boolean;
begin
  if WizardSilent then Exit;
  Bienvenida := CurPageID = wpWelcome;
  Instalando := (CurPageID = wpInstalling) or (CurPageID = wpPreparing);
  Listo := CurPageID = wpFinished;
  if Bienvenida then Fondear('bienvenida')
  else if Instalando then Fondear('instalando')
  else if Listo then Fondear('listo');

  Mostrar(Instalar, Bienvenida);
  Mostrar(Cancelar, Bienvenida);
  Mostrar(Cambiar, Bienvenida);
  RutaTexto.Visible := Bienvenida;
  Mostrar(CancelarInstalando, Instalando);
  EstadoTexto.Visible := Instalando;
  CifraTexto.Visible := Instalando;
  Relleno.Visible := Instalando;
  Mostrar(Terminar, Listo);
  AvisoTexto.Visible := Listo and (Aviso <> '');
  AvisoTexto.Caption := Aviso;
  PintarCasillas;
  Mostrar(Cerrar, True);
  // The last screen closes the window: stop dragging before that happens.
  if Listo then SoltarArrastre;

  if Instalando and (EstadoTexto.Caption = ' ') then
  begin
    EstadoTexto.Caption := SetupMessage(msgStatusExtractFiles);
    CifraTexto.Caption := '0 %';
    Relleno.Width := 0;
  end;
  EsconderAsistente;
end;

procedure CurInstallProgressChanged(CurProgress, MaxProgress: Integer);
begin
  if (MaxProgress <= 0) or (Relleno = nil) then Exit;
  Relleno.Width := ScaleX({#relleno_W}) * CurProgress div MaxProgress;
  CifraTexto.Caption := IntToStr(CurProgress * 100 div MaxProgress) + ' %';
end;

{ Wireless Display: Windows downloads it, and Reflex reads the marker in %LOCALAPPDATA%\Reflex
  (src/miracast.rs) to tell "restart needed" from "missing". Setup runs elevated, so the marker is
  written as the user who launched it, into their own profile. }

procedure AnadirPantalla;
var
  Codigo, Nada: Integer;
begin
  WizardForm.StatusLabel.Caption := CustomMessage('Pantalla');
  if EstadoTexto <> nil then EstadoTexto.Caption := CustomMessage('Pantalla');
  if not Exec(ExpandConstant('{sys}\dism.exe'), '/Online /Add-Capability /CapabilityName:App.WirelessDisplay.Connect~~~~0.0.1.0 /NoRestart',
    '', SW_HIDE, ewWaitUntilTerminated, Codigo) then Codigo := -1;
  Log('DISM Wireless Display: ' + IntToStr(Codigo));
  if (Codigo = 0) or (Codigo = 3010) then
  begin
    ExecAsOriginalUser(ExpandConstant('{cmd}'), '/c (if not exist "%LOCALAPPDATA%\Reflex" mkdir "%LOCALAPPDATA%\Reflex") & echo %DATE% %TIME%> "%LOCALAPPDATA%\Reflex\pantalla_inalambrica_instalada.txt"',
      '', SW_HIDE, ewWaitUntilTerminated, Nada);
    Aviso := CustomMessage('Reiniciar');
  end
  else
    Aviso := CustomMessage('PantallaFallo');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and QuiereInalambrica then AnadirPantalla;
end;

procedure CancelButtonClick(CurPageID: Integer; var Cancel, Confirm: Boolean);
begin
  if WizardSilent then Exit;
  // Asked here, so the hooks are released only when leaving for real.
  Confirm := False;
  Cancel := MsgBox(CustomMessage('Salir'), mbConfirmation, MB_YESNO) = IDYES;
  if Cancel then Soltar;
end;

procedure DeinitializeSetup;
begin
  if Temporizador <> 0 then KillTimer(0, Temporizador);
  Temporizador := 0;
end;
