# toggle_mode.ps1 - Fast Switcher between WORK MODE and GAMER MODE with Audio Feedback
param(
    [string]$TargetMode = ""
)

$scriptsDir = if ($env:USERPROFILE) { "$env:USERPROFILE\scripts" } else { "C:\scripts" }
$stateFile = "$scriptsDir\current_mode.txt"
$rainmeterExe = "C:\Program Files\Rainmeter\Rainmeter.exe"
$dockerExe = "C:\Program Files\Docker\Docker\Docker Desktop.exe"
$ollamaExe = "$env:LOCALAPPDATA\Programs\Ollama\ollama app.exe"
$iconWork = "$scriptsDir\coffee.ico"
$iconGamer = "$scriptsDir\gamer.ico"
$soundGamer = "$scriptsDir\hadouken.wav"
$soundWork = "$scriptsDir\theoffice.wav"

if (-not (Test-Path $scriptsDir)) {
    New-Item -ItemType Directory -Path $scriptsDir -Force | Out-Null
}

$currentMode = "WORK"
if (Test-Path $stateFile) {
    $currentMode = (Get-Content $stateFile -Raw).Trim()
}

$nextMode = if ($TargetMode -ne "") { $TargetMode.ToUpper() } else { if ($currentMode -eq "WORK") { "GAMER" } else { "WORK" } }

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

# Play Audio Feedback Immediately (Asynchronously)
try {
    $soundFile = if ($nextMode -eq "GAMER") { $soundGamer } else { $soundWork }
    if (Test-Path $soundFile) {
        $player = New-Object System.Media.SoundPlayer $soundFile
        $player.Play()
    }
} catch {
    # Ignore audio error if device busy
}

if ($nextMode -eq "GAMER") {
    Write-Host "[GAMER MODE] Desativando Docker, WSL e Ollama para foco 100% na GPU..."

    # 1. Kill Ollama
    Get-Process -Name "ollama*", "llama-server" -ErrorAction SilentlyContinue | Stop-Process -Force

    # 2. Kill Docker Desktop
    Get-Process -Name "*docker*", "wslrelay" -ErrorAction SilentlyContinue | Stop-Process -Force
    Stop-Service -Name "com.docker.service" -ErrorAction SilentlyContinue

    # 3. Shutdown WSL2 completely (releases all hypervisor RAM and CPU)
    & wsl.exe --shutdown

    # 4. Activate Ultimate Performance Power Plan
    & powercfg -setactive 872d9590-9f19-44a3-87f5-c9cbd49ea59e

    # 5. Set State
    Set-Content -Path $stateFile -Value "GAMER" -Force

    # 6. Toast / Notification
    $notify = New-Object System.Windows.Forms.NotifyIcon
    if (Test-Path $iconGamer) {
        $notify.Icon = New-Object System.Drawing.Icon($iconGamer)
    } else {
        $notify.Icon = [System.Drawing.SystemIcons]::Application
    }
    $notify.BalloonTipTitle = "MODO GAMER ATIVADO"
    $notify.BalloonTipText = "Docker, WSL e Ollama encerrados. VRAM da RTX e CPU 100% liberados para o jogo!"
    $notify.Visible = $true
    $notify.ShowBalloonTip(3000)

    Write-Host "MODO GAMER ATIVADO COM SUCESSO."

} else {
    Write-Host "[WORK MODE] Restaurando Docker, WSL e Ollama para desenvolvimento..."

    # 1. Start Ollama
    if (Test-Path $ollamaExe) {
        Start-Process -FilePath $ollamaExe
    }

    # 2. Start Docker Service & Desktop
    Start-Service -Name "com.docker.service" -ErrorAction SilentlyContinue
    if (Test-Path $dockerExe) {
        Start-Process -FilePath $dockerExe
    }

    # 3. Initialize WSL
    Start-Process -FilePath "wsl.exe" -ArgumentList "-d", "Ubuntu", "-e", "true"

    # 4. Set State
    Set-Content -Path $stateFile -Value "WORK" -Force

    # 5. Toast / Notification
    $notify = New-Object System.Windows.Forms.NotifyIcon
    if (Test-Path $iconWork) {
        $notify.Icon = New-Object System.Drawing.Icon($iconWork)
    } else {
        $notify.Icon = [System.Drawing.SystemIcons]::Information
    }
    $notify.BalloonTipTitle = "MODO TRABALHO ATIVADO"
    $notify.BalloonTipText = "Docker, WSL e Ollama iniciados. Ambiente de programacao e IA pronto."
    $notify.Visible = $true
    $notify.ShowBalloonTip(3000)

    Write-Host "MODO WORK ATIVADO COM SUCESSO."
}

# Refresh Rainmeter widget
if (Test-Path $rainmeterExe) {
    & $rainmeterExe "!Refresh" "Simplic\ModeSwitch"
}
