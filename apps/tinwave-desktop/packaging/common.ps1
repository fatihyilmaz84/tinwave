# Shared helpers for the Windows packaging scripts (dot-source this file). See README.md.
#
# Nothing here touches the machine outside the process: no registry, no system PATH, no installs.
Set-StrictMode -Version Latest

$script:PackagingDir = $PSScriptRoot
$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path

# --- PE import table reader ------------------------------------------------------------------------------------------
# The static-Qt and "is it self-contained" checks read the import table of every PE file in the bundle. Plain C# via
# Add-Type: byte-offset walking in PowerShell would be slow on 60 DLLs.
if (-not ('Tw.PeImports' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;

namespace Tw {
    public sealed class PeImports {
        public bool Is64Bit;
        public List<string> Imports = new List<string>();      // regular import table
        public List<string> DelayImports = new List<string>(); // delay-load import table

        public static PeImports Read(string path) {
            byte[] d = File.ReadAllBytes(path);
            if (d.Length < 0x40 || d[0] != 'M' || d[1] != 'Z') throw new InvalidDataException(path + ": not a PE file");
            int pe = BitConverter.ToInt32(d, 0x3c);
            if (pe < 0 || pe + 24 > d.Length || d[pe] != 'P' || d[pe + 1] != 'E') throw new InvalidDataException(path + ": no PE header");
            int nSections = BitConverter.ToUInt16(d, pe + 6);
            int optSize = BitConverter.ToUInt16(d, pe + 20);
            int opt = pe + 24;
            ushort magic = BitConverter.ToUInt16(d, opt);
            var r = new PeImports();
            r.Is64Bit = magic == 0x20b;
            if (magic != 0x20b && magic != 0x10b) throw new InvalidDataException(path + ": unknown optional header");
            int ddBase = opt + (r.Is64Bit ? 112 : 96);
            int sec = opt + optSize;
            uint[] va = new uint[nSections], vs = new uint[nSections], raw = new uint[nSections];
            for (int i = 0; i < nSections; i++) {
                int s = sec + i * 40;
                vs[i] = BitConverter.ToUInt32(d, s + 8);
                va[i] = BitConverter.ToUInt32(d, s + 12);
                raw[i] = BitConverter.ToUInt32(d, s + 20);
            }
            Func<uint, int> off = rva => {
                for (int i = 0; i < nSections; i++)
                    if (rva >= va[i] && rva < va[i] + Math.Max(vs[i], 1u)) return (int)(rva - va[i] + raw[i]);
                return -1;
            };
            Func<uint, string> str = rva => {
                int o = off(rva);
                if (o < 0) return null;
                int e = o;
                while (e < d.Length && d[e] != 0) e++;
                return Encoding.ASCII.GetString(d, o, e - o);
            };
            uint impRva = BitConverter.ToUInt32(d, ddBase + 1 * 8);
            int io = impRva == 0 ? -1 : off(impRva);
            while (io >= 0 && io + 20 <= d.Length) {
                uint nameRva = BitConverter.ToUInt32(d, io + 12);
                if (nameRva == 0) break;
                string n = str(nameRva);
                if (n != null) r.Imports.Add(n);
                io += 20;
            }
            uint delRva = BitConverter.ToUInt32(d, ddBase + 13 * 8);
            int dof = delRva == 0 ? -1 : off(delRva);
            while (dof >= 0 && dof + 32 <= d.Length) {
                uint attrs = BitConverter.ToUInt32(d, dof);
                uint nameRef = BitConverter.ToUInt32(d, dof + 4);
                if (nameRef == 0) break;
                // Attribute bit 0 set: RVAs (modern images). Clear: virtual addresses (not produced by MSVC x64).
                string n = (attrs & 1) != 0 ? str(nameRef) : null;
                if (n != null) r.DelayImports.Add(n);
                dof += 32;
            }
            return r;
        }
    }
}
'@
}

function Get-PeImports([string]$Path) { [Tw.PeImports]::Read($Path) }

# --- Qt kit ----------------------------------------------------------------------------------------------------------
function Resolve-QtKit([string]$QtDir) {
    # -QtDir wins; otherwise the kit that QMAKE points at (the documented build environment).
    if (-not $QtDir) {
        if (-not $env:QMAKE) { throw "No Qt kit: pass -QtDir or set QMAKE to the kit's qmake.exe (docs/guides/desktop-dev-setup.md)." }
        $QtDir = Split-Path (Split-Path $env:QMAKE -Parent) -Parent
    }
    $root = (Resolve-Path $QtDir).Path
    $bin = Join-Path $root 'bin'
    foreach ($tool in 'qmake.exe', 'windeployqt.exe') {
        if (-not (Test-Path (Join-Path $bin $tool))) { throw "Not a Qt kit: $bin\$tool is missing." }
    }
    $version = (& (Join-Path $bin 'qmake.exe') -query QT_VERSION).Trim()
    if (-not $version) { throw "qmake -query QT_VERSION returned nothing for $root." }
    # <QtRoot>\<version>\<kit>: the Licenses and Docs folders sit next to the version folder.
    $qtRoot = Split-Path (Split-Path $root -Parent) -Parent
    [pscustomobject]@{ Root = $root; Bin = $bin; Version = $version; QtRoot = $qtRoot }
}

function Get-QtConfigValue([string]$Kit, [string]$Name) {
    # mkspecs\qconfig.pri: `QT.global.enabled_features = a b c`.
    $pri = Join-Path $Kit 'mkspecs\qconfig.pri'
    if (-not (Test-Path $pri)) { throw "Qt kit has no mkspecs\qconfig.pri ($pri)." }
    foreach ($line in Get-Content $pri) {
        if ($line -match ('^\s*' + [regex]::Escape($Name) + '\s*[+]?=\s*(.*)$')) { return ($Matches[1] -split '\s+') }
    }
    return @()
}

# --- Qt SBOM (the kit ships SPDX 2.3 JSON SBOMs per module repo) -----------------------------------------------------
function Read-QtSbom([string]$Kit) {
    $dir = Join-Path $Kit 'sbom'
    $files = @(Get-ChildItem $dir -Filter '*.spdx.json' -ErrorAction SilentlyContinue | Where-Object { $_.Name -notlike '*.source.*' })
    if ($files.Count -eq 0) { throw "No SPDX SBOMs under $dir; the license check cannot be done." }
    $pkgs = @{}
    foreach ($f in $files) {
        $doc = [IO.File]::ReadAllText($f.FullName, [Text.Encoding]::UTF8) | ConvertFrom-Json
        foreach ($p in $doc.packages) {
            $pkgs[$p.name] = [pscustomobject]@{
                Name      = $p.name
                Version   = $p.versionInfo
                License   = [string]$p.licenseConcluded
                Copyright = [string]$p.copyrightText
                Comment   = [string]$p.comment
                Download  = [string]$p.downloadLocation
                Repo      = $f.Name -replace '-[\d.]+\.spdx\.json$', ''
            }
        }
    }
    $pkgs
}

# A Qt license expression such as `LicenseRef-Qt-Commercial OR LGPL-3.0-only OR GPL-2.0-only OR GPL-3.0-only`.
# "GPL-only" = every non-commercial alternative is a GPL one (no LGPL, no permissive choice): we may not ship it.
function Test-GplOnly([string]$Expression) {
    if (-not $Expression -or $Expression -eq 'NOASSERTION') { return $false }   # not a Qt module (build tools, ...)
    $terms = @($Expression -split '\s+(?:OR|AND)\s+' | ForEach-Object { ($_ -replace '\s+WITH\s+.*$', '').Trim() })
    $usable = @($terms | Where-Object { $_ -notmatch '^LicenseRef-Qt-Commercial$' -and $_ -notmatch '^GPL-' })
    return ($usable.Count -eq 0)
}

# Map a Qt DLL in the bundle to the SBOM packages that describe it. Returns the matching packages (possibly none).
function Find-QtSbomPackages([string]$DllPath, $Sbom) {
    $stem = [IO.Path]::GetFileNameWithoutExtension($DllPath)
    if ($stem -match '^Qt6(.+)$') {
        $m = $Matches[1]
        if ($Sbom.ContainsKey($m)) { return @($Sbom[$m]) }
        return @()
    }
    $s = $stem.ToLowerInvariant()
    # Plugins whose file name is not a prefix of the SBOM package name (QtNetwork's NLMNI / TLS backends).
    $alias = @{ 'qnetworklistmanager' = 'QNLMNIPlugin'; 'qcertonlybackend' = 'QTlsBackendCertOnlyPlugin'; 'qopensslbackend' = 'QTlsBackendOpenSSLPlugin' }
    if ($alias.ContainsKey($s)) { if ($Sbom.ContainsKey($alias[$s])) { return @($Sbom[$alias[$s]]) } else { return @() } }
    # Tiers: exact name; equal once a trailing "plugin" is dropped; the SBOM name starts with the file stem (qwindows ->
    # QWindowsIntegrationPlugin). The first tier with a hit wins.
    $all = @($Sbom.Values | Where-Object { $_.Name -notmatch '_Attribution_' })
    $t1 = @($all | Where-Object { $_.Name.ToLowerInvariant() -eq $s })
    if ($t1.Count -gt 0) { return $t1 }
    $sNo = $s -replace 'plugin$', ''
    $t2 = @($all | Where-Object { ($_.Name.ToLowerInvariant() -replace 'plugin$', '') -eq $sNo })
    if ($t2.Count -gt 0) { return $t2 }
    @($all | Where-Object { $_.Name.ToLowerInvariant().StartsWith($s) })
}

# --- Bundle inventory ------------------------------------------------------------------------------------------------
function Get-BundleFiles([string]$Bundle) {
    $root = (Resolve-Path $Bundle).Path.TrimEnd('\')
    Get-ChildItem $root -Recurse -File | ForEach-Object {
        [pscustomobject]@{ Path = $_.FullName; Rel = $_.FullName.Substring($root.Length + 1); Name = $_.Name; Length = $_.Length }
    }
}

function Get-BundleSummary([string]$Bundle) {
    $f = @(Get-BundleFiles $Bundle)
    $sum = ($f | Measure-Object -Property Length -Sum).Sum
    [pscustomobject]@{ Files = $f.Count; Bytes = [long]$sum; MiB = [math]::Round($sum / 1MB, 1) }
}

# --- Text I/O --------------------------------------------------------------------------------------------------------
function Read-TextFile([string]$Path) {
    # Strict UTF-8 first; Qt's attribution pages and some crate licenses are Latin-1/Windows-1252.
    $bytes = [IO.File]::ReadAllBytes($Path)
    try {
        $enc = New-Object Text.UTF8Encoding($false, $true)
        $t = $enc.GetString($bytes)
    } catch {
        $t = [Text.Encoding]::GetEncoding(1252).GetString($bytes)
    }
    if ($t.Length -gt 0 -and $t[0] -eq [char]0xFEFF) { $t = $t.Substring(1) }
    $t -replace "`r`n", "`n"
}

function Write-TextFile([string]$Path, [string]$Text) {
    $dir = Split-Path $Path -Parent
    if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
    [IO.File]::WriteAllText($Path, $Text, (New-Object Text.UTF8Encoding($false)))
}
