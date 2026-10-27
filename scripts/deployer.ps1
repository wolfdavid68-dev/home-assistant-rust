<#
.SYNOPSIS
    Construit l'image de ha-server sur cette machine et la met en service,
    avec retour automatique à l'image d'avant si elle ne démarre pas.

.DESCRIPTION
    Pour qui construit l'image chez soi (Docker Desktop sous Windows, par
    exemple) au lieu de prendre celle de ghcr.io. Dans l'ordre :

      1. l'image en service reçoit une étiquette de secours
         (<image>-avant-<date>) : on peut toujours y revenir ;
      2. docker build, depuis ce dépôt ;
      3. docker compose up -d --no-build <service>, dans le dossier compose ;
      4. attente de l'état « healthy » (le HEALTHCHECK de l'image) ;
      5. si le service n'est pas « healthy » à temps : l'image de secours
         reprend l'étiquette, et le service redémarre avec elle.

    Le fichier compose du dossier doit utiliser la même image que -Image
    (« image: ha-server:local », comme le compose.yaml du dépôt). Ni la
    base, ni la clé, ni les volumes ne sont touchés : seule l'image change.

    Guide : docs/installation.md, « Construire l'image sur sa machine ».

.PARAMETER Dossier
    Le dossier qui contient le compose.yaml de la maison (et son .env).

.PARAMETER Image
    L'image utilisée par ce compose.yaml, par exemple ha-server:local.

.PARAMETER Service
    Le service à redémarrer (ha-server par défaut).

.PARAMETER Source
    Le dépôt à construire (par défaut, celui qui contient ce script).

.PARAMETER Secours
    Le mot de l'étiquette de secours (par défaut la date et l'heure,
    par exemple 20261027-1530). Lettres, chiffres, « . », « _ » et « - ».

.PARAMETER Delai
    Combien de secondes attendre l'état « healthy » (180 par défaut).

.EXAMPLE
    .\scripts\deployer.ps1 -Dossier D:\maison -Image ha-server:local
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Dossier,

    [Parameter(Mandatory = $true)]
    [string] $Image,

    [string] $Service = 'ha-server',

    [string] $Source = (Split-Path -Parent $PSScriptRoot),

    [ValidatePattern('^[A-Za-z0-9_.-]{1,40}$')]
    [string] $Secours = (Get-Date -Format 'yyyyMMdd-HHmm'),

    [ValidateRange(30, 3600)]
    [int] $Delai = 180
)

$ErrorActionPreference = 'Stop'

function Etape([string] $Texte) {
    Write-Host ''
    Write-Host "== $Texte" -ForegroundColor Cyan
}

# Lance une commande docker ; arrête le script si elle échoue.
function Lancer-Docker {
    & docker @args
    if ($LASTEXITCODE -ne 0) {
        throw "La commande « docker $($args -join ' ') » a échoué (code $LASTEXITCODE)."
    }
}

# L'image existe-t-elle déjà ? (Windows PowerShell 5.1 transformerait le
# message d'erreur de docker en exception : on le laisse passer ici.)
function Image-Existe([string] $Nom) {
    $avant = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & docker image inspect $Nom 2>$null | Out-Null
        return ($LASTEXITCODE -eq 0)
    } finally {
        $ErrorActionPreference = $avant
    }
}

# L'image avec son étiquette : « nom:etiquette » (« latest » si rien).
function Avec-Etiquette([string] $Nom) {
    $dernier = $Nom.Split('/')[-1]
    if ($dernier.Contains(':')) { return $Nom }
    return "${Nom}:latest"
}

# Attend que le conteneur du service soit « healthy ». Rend $true ou $false.
function Attendre-Sante {
    $fin = (Get-Date).AddSeconds($Delai)
    $etat = 'inconnu'
    while ((Get-Date) -lt $fin) {
        $id = (& docker compose ps -q $Service | Select-Object -First 1)
        if ($id) {
            $etat = (& docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' $id)
            if ($etat -eq 'healthy') { return $true }
            if ($etat -in @('unhealthy', 'exited', 'dead')) {
                Write-Host "Le service est « $etat »." -ForegroundColor Yellow
                return $false
            }
        }
        Write-Host "  … $etat"
        Start-Sleep -Seconds 5
    }
    Write-Host "Toujours « $etat » après $Delai s." -ForegroundColor Yellow
    return $false
}

$Image = Avec-Etiquette $Image
$secours = "$Image-avant-$Secours"
$Dossier = (Resolve-Path $Dossier).Path
$Source = (Resolve-Path $Source).Path

if (-not (Test-Path (Join-Path $Source 'Dockerfile'))) {
    throw "Pas de Dockerfile dans $Source : indique le dépôt avec -Source."
}

Push-Location $Dossier
try {
    Etape 'Vérifications'
    Lancer-Docker version --format 'Docker {{.Server.Version}}'
    $images = @(& docker compose config --images | ForEach-Object { Avec-Etiquette $_ })
    if ($LASTEXITCODE -ne 0) { throw "Le fichier compose de $Dossier n'est pas valide." }
    if ($images -notcontains $Image) {
        throw "Le fichier compose de $Dossier n'utilise pas l'image $Image (il utilise : $($images -join ', ')). Mets « image: $Image » au service $Service."
    }
    if (Get-Command git -ErrorAction SilentlyContinue) {
        $commit = (& git -C $Source log -1 --format '%h %s')
        Write-Host "Dépôt : $commit"
        if (& git -C $Source status --porcelain) {
            Write-Host 'Attention : le dépôt contient des changements non enregistrés (git status).' -ForegroundColor Yellow
        }
    }

    Etape "Étiquette de secours : $secours"
    $aSecours = Image-Existe $Image
    if ($aSecours) {
        Lancer-Docker tag $Image $secours
        Write-Host "L'image en service est gardée sous $secours."
    } else {
        Write-Host "Pas encore d'image $Image : premier déploiement, sans secours."
    }

    Etape "Construction de $Image"
    Lancer-Docker build -t $Image $Source

    Etape "Mise en service de $Service"
    Lancer-Docker compose up -d --no-build $Service

    Etape 'Attente du démarrage'
    if (Attendre-Sante) {
        Write-Host ''
        Write-Host "C'est en service : $Service tourne avec la nouvelle image." -ForegroundColor Green
        if ($aSecours) {
            Write-Host "Pour revenir en arrière : docker tag $secours $Image, puis docker compose up -d --no-build $Service"
            Write-Host "Pour libérer la place plus tard : docker image rm $secours"
        }
        exit 0
    }

    & docker compose logs --tail 40 $Service
    if (-not $aSecours) {
        throw "La nouvelle image ne démarre pas, et il n'y a pas d'image d'avant pour revenir en arrière."
    }

    Etape "Retour à l'image d'avant ($secours)"
    Lancer-Docker tag $secours $Image
    Lancer-Docker compose up -d --no-build $Service
    if (Attendre-Sante) {
        Write-Host "La nouvelle image ne démarrait pas : $Service est revenu à l'image d'avant." -ForegroundColor Yellow
    } else {
        Write-Host "Même l'image d'avant ne démarre pas : regarde « docker compose logs $Service »." -ForegroundColor Red
    }
    exit 1
} finally {
    Pop-Location
}
