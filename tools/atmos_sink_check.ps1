# Send the three titles Dolby's object path refuses to a real Atmos renderer.
#
# The open question is why Dolby's software refuses the object presentation of
# Shaun of the Dead, Knives Out and Kingsman while opening Pi, Talk to Me and
# Braveheart. Two things are already known: a third decoder, `truehdd`, opens all
# six with byte-identical object audio, and the field that correlates across the
# six is not sufficient to cause the refusal. What is not known is whether a
# renderer that is neither of those -- a piece of hardware that has to light up
# an "Atmos" lamp or not -- agrees with Dolby.
#
# This script sends each stream to the sink as a bitstream and lets the sink
# decode it. There is no return path for what a sink decided: it says so on its
# own front panel and nowhere else. So this needs a person watching the display,
# and the result it produces is a yes or a no, not a measurement.
#
# Run it deliberately. It plays audio at whatever volume the sink is set to.
#
#   pwsh tools/atmos_sink_check.ps1                 # list the devices and stop
#   pwsh tools/atmos_sink_check.ps1 -Play           # play each clip in turn
#   pwsh tools/atmos_sink_check.ps1 -Play -Seconds 30
#
# What to write down for each title: what the sink's display names the format
# (Dolby TrueHD, Dolby Atmos, PCM, nothing), and whether it changes between the
# accepted and the refused set.

param(
    [switch]$Play,
    [int]$Seconds = 20,
    [string]$Device = '',
    [string]$Media = 'E:\oadec-work\thd'
)

$mpv = 'C:\Program Files\mpv\mpv.exe'
if (-not (Test-Path $mpv)) { Write-Error "mpv is not at $mpv"; exit 2 }

if (-not $Device) {
    Write-Host 'Audio devices mpv can see:' -ForegroundColor Cyan
    & $mpv --audio-device=help 2>&1 | Select-String -Pattern 'wasapi' | ForEach-Object { "  $_" }
    Write-Host ''
    Write-Host 'Pass the one that is the Atmos sink as -Device, for example:' -ForegroundColor Cyan
    Write-Host "  pwsh tools/atmos_sink_check.ps1 -Play -Device 'wasapi/{...}'"
    if ($Play) { Write-Error 'no -Device given'; exit 2 }
    exit 0
}

# Dolby's object path opens the first three and refuses the last three.
$titles = @(
    @{ name = 'Pi';                file = 'pi.thd';         dolby = 'accepted' },
    @{ name = 'Talk to Me';        file = 'talktome.thd';   dolby = 'accepted' },
    @{ name = 'Braveheart';        file = 'braveheart.thd'; dolby = 'accepted' },
    @{ name = 'Shaun of the Dead'; file = 'shaun.thd';      dolby = 'refused'  },
    @{ name = 'Knives Out';        file = 'knivesout.thd';  dolby = 'refused'  },
    @{ name = 'Kingsman';          file = 'kingsman.thd';   dolby = 'refused'  }
)

foreach ($t in $titles) {
    $path = Join-Path $Media $t.file
    if (-not (Test-Path $path)) { Write-Host "  skip $($t.name): $path is not there" -ForegroundColor DarkYellow; continue }
    Write-Host ''
    Write-Host "$($t.name)  --  Dolby's object path: $($t.dolby)" -ForegroundColor Green
    Write-Host "  watch the sink's display, then press a key" -ForegroundColor DarkGray
    if (-not $Play) { continue }
    & $mpv --no-video --audio-device=$Device --audio-exclusive=yes --audio-spdif=truehd `
        --start=120 --length=$Seconds --really-quiet $path
    Write-Host '  what did the display say? (note it, then press Enter)' -NoNewline
    [void][System.Console]::ReadLine()
}
