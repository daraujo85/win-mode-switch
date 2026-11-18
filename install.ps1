# install.ps1 - One-Click Installer for Win Mode Switch
[CmdletBinding()]
param(
    [switch]$NoRainmeter
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "   WIN MODE SWITCH - INSTALADOR / SETUP" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

$userProfile = $env:USERPROFILE
$targetScriptsDir = "$userProfile\scripts"
$startupDir = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup"
$desktopDir = "$userProfile\Desktop"
$sourceDir = $PSScriptRoot

if (-not (Test-Path $targetScriptsDir)) {
    New-Item -ItemType Directory -Path $targetScriptsDir -Force | Out-Null
    Write-Host "[+] Criado diretorio: $targetScriptsDir" -ForegroundColor Green
}

# 1. Copiar scripts e assets
Write-Host "[*] Copiando scripts, icones e audios para $targetScriptsDir..." -ForegroundColor Yellow
Copy-Item -Path "$sourceDir\scripts\toggle_mode.ps1" -Destination $targetScriptsDir -Force
Copy-Item -Path "$sourceDir\assets\sounds\*.wav" -Destination $targetScriptsDir -Force
Copy-Item -Path "$sourceDir\assets\icons\*.ico" -Destination $targetScriptsDir -Force
Copy-Item -Path "$sourceDir\assets\icons\*.png" -Destination $targetScriptsDir -Force

# 2. Copiar binario modeswitch.exe
$binSource = if (Test-Path "$sourceDir\target\release\modeswitch.exe") {
    "$sourceDir\target\release\modeswitch.exe"
} elseif (Test-Path "$sourceDir\modeswitch.exe") {
    "$sourceDir\modeswitch.exe"
} else {
    $null
}

if ($binSource) {
    Stop-Process -Name "modeswitch" -Force -ErrorAction SilentlyContinue
    Copy-Item -Path $binSource -Destination "$targetScriptsDir\modeswitch.exe" -Force
    Write-Host "[+] Executavel modeswitch.exe copiado com sucesso." -ForegroundColor Green
} else {
    Write-Host "[!] Executavel modeswitch.exe nao encontrado na raiz. Compile com 'cargo build --release' se desejar o app nativo de bandeja." -ForegroundColor Yellow
}

# 3. Criar Atalho na Area de Trabalho
Write-Host "[*] Criando atalho na Area de Trabalho..." -ForegroundColor Yellow
$sh = New-Object -ComObject WScript.Shell
$desktopLnk = $sh.CreateShortcut("$desktopDir\Alternar Modo Gamer e Work.lnk")
$desktopLnk.TargetPath = "powershell.exe"
$desktopLnk.Arguments = "-ExecutionPolicy Bypass -WindowStyle Hidden -File `"$targetScriptsDir\toggle_mode.ps1`""
$desktopLnk.WorkingDirectory = $targetScriptsDir
if (Test-Path "$targetScriptsDir\mode_switch.ico") {
    $desktopLnk.IconLocation = "$targetScriptsDir\mode_switch.ico,0"
}
$desktopLnk.Save()
Write-Host "[+] Atalho 'Alternar Modo Gamer e Work.lnk' criado na Area de Trabalho!" -ForegroundColor Green

# 4. Criar Atalho na Inicializacao do Windows (Startup)
if (Test-Path "$targetScriptsDir\modeswitch.exe") {
    Write-Host "[*] Configurando inicializacao automatica com o Windows..." -ForegroundColor Yellow
    $startupLnk = $sh.CreateShortcut("$startupDir\modeswitch.lnk")
    $startupLnk.TargetPath = "$targetScriptsDir\modeswitch.exe"
    $startupLnk.WorkingDirectory = $targetScriptsDir
    $startupLnk.Save()
    Write-Host "[+] modeswitch configurado no Startup!" -ForegroundColor Green
}

# 5. Instalar Skin do Rainmeter (opcional)
$rainmeterSkins = "$userProfile\Documents\Rainmeter\Skins"
if ((-not $NoRainmeter) -and (Test-Path $rainmeterSkins) -and (Test-Path "$sourceDir\rainmeter\ModeSwitch")) {
    Write-Host "[*] Rainmeter detectado. Instalando skin ModeSwitch..." -ForegroundColor Yellow
    $destSkin = "$rainmeterSkins\Simplic\ModeSwitch"
    if (-not (Test-Path $destSkin)) {
        New-Item -ItemType Directory -Path $destSkin -Force | Out-Null
    }
    Copy-Item -Path "$sourceDir\rainmeter\ModeSwitch\*" -Destination $destSkin -Recurse -Force
    Write-Host "[+] Skin ModeSwitch instalada em $destSkin" -ForegroundColor Green
}

# 6. Iniciar modeswitch.exe
if (Test-Path "$targetScriptsDir\modeswitch.exe") {
    Start-Process -FilePath "$targetScriptsDir\modeswitch.exe" -WorkingDirectory $targetScriptsDir
    Write-Host "[+] modeswitch.exe iniciado na bandeja do sistema!" -ForegroundColor Green
}

Write-Host "`n>>> INSTALACAO CONCLUIDA COM SUCESSO! <<<" -ForegroundColor Green
Write-Host "Modo Gamer: Desativa Docker, WSL e Ollama, aplica plano de energia maximo e toca HADOUKEN!"
Write-Host "Modo Work : Inicia Docker, WSL e Ollama e toca o tema de THE OFFICE!"
