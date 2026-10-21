@echo off
rem Lance le serveur a chaque ouverture de session Windows (fenetre reduite).
rem Pour l'enlever : schtasks /Delete /TN "Maison ha-server" /F

schtasks /Create /TN "Maison ha-server" /SC ONLOGON /RL LIMITED /F ^
  /TR "cmd /c start \"Maison\" /min \"%~dp0demarrer.bat\""
if errorlevel 1 (
  echo Echec : relance ce fichier, ou vois le guide d'installation.
) else (
  echo C'est fait : le serveur demarrera a chaque ouverture de session.
)
pause
