@echo off
rem Demarre le serveur de la maison (home-assistant-rust).
rem Les donnees (ha.db, ha.key, sauvegardes) restent dans ce dossier.
rem Guide : https://github.com/wolfdavid68-dev/home-assistant-rust/blob/main/docs/installation.md

cd /d "%~dp0"

rem Adresse d'ecoute : seulement ce PC. Pour ton reseau local (telephone,
rem tablette), remplace par 0.0.0.0:8123 et accepte le pare-feu Windows
rem pour les reseaux PRIVES seulement. Jamais de redirection depuis la box.
set HA_BIND=127.0.0.1:8123

rem Sauvegardes chaque nuit dans le dossier « sauvegardes » (7 gardees).
set HA_BACKUP_DIR=%~dp0sauvegardes

rem Position de la maison (soleil, meteo, protection solaire) :
rem set HA_LATITUDE=48.85
rem set HA_LONGITUDE=2.35

rem Enceintes Sonos et annonces vocales (voir docs/sonos.md) : Piper et sa
rem voix dans le dossier « piper », a cote de ha-server.exe.
rem set HA_SONOS=auto
rem set HA_PIPER=%~dp0piper\piper.exe

rem Fuseau horaire et niveau de detail du journal :
set TZ=Europe/Paris
set RUST_LOG=info

ha-server.exe
if errorlevel 1 pause
