; Compile with Inno Setup on Windows after building/staging the plugin.
; Example: ISCC /DPluginSource="C:\build\SoniclawReference.vst3" Soniclaw.iss
#ifndef PluginSource
  #error Define PluginSource as the built VST3 bundle directory
#endif
[Setup]
AppId=org.sonora.reference
AppName=Soniclaw Reference
AppVersion=0.3.0
DefaultDirName={commoncf64}\VST3
DisableDirPage=yes
UninstallFilesDir={commonappdata}\Soniclaw\Reference
OutputBaseFilename=SoniclawReference-0.3.0-Windows
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
Compression=lzma2
SolidCompression=yes
[Files]
Source: "{#PluginSource}\*"; DestDir: "{app}\SoniclawReference.vst3"; Flags: ignoreversion recursesubdirs createallsubdirs
