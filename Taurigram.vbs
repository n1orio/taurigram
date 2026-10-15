Set WshShell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
scriptDir = fso.GetParentFolderName(WScript.ScriptFullName)
clientDir = fso.BuildPath(scriptDir, "client")
appExe = fso.BuildPath(clientDir, "tauri\target\debug\taurigram.exe")

WshShell.CurrentDirectory = clientDir
WshShell.Run """" & appExe & """", 0, False
