# LuxMini 0.5.2

Version ciblée sur la luminosité de veille. Les chantiers de compteurs et de
parcours statistiques restent dans leur branche et ne sont pas inclus.

## Interface et comportement

Dans Réglages → LED, la section Mac sleep porte un petit badge BETA et un bouton
Report a bug. L'interrupteur est désactivé par défaut, le curseur va de 0 à 100 %
et 0 % signifie éteint. Les changements sont enregistrés immédiatement ; ils
ne modifient pas la luminosité courante. Le bouton ouvre un brouillon GitHub
avec version, modèle et réglage, sans envoyer automatiquement de rapport.

Seule la veille du système applique cette luminosité fixe. Les effets sont
suspendus, les préférences manuelles et presets sont conservés, puis les règles
sont recalculées au réveil avec l'heure actuelle. Sans règle active, le réglage
manuel et l'effet reprennent. La priorité manuelle de 90 minutes est conservée.
L'écran éteint et le verrouillage seuls ne déclenchent pas cette fonction.

Les commandes concurrentes ne remplacent pas la valeur de veille et l'API locale
ne déclare pas une commande ignorée comme appliquée. Les notifications répétées
sont idempotentes et les erreurs de pilote suivent le traitement existant.

Les raccourcis natifs Couper, Copier, Coller et Tout sélectionner sont aussi
rétablis dans les champs des réglages.

## Validation

- 129 tests réussis en configuration normale ; 130 en Field Test. Deux tests
  matériels ignorés dans chaque configuration.
- Clippy strict dans les deux configurations, formatage et shellcheck.
- 20 scénarios de consentement par processus sur le client livré, avec
  préférences jetables et serveur local. Aucun changement du protocole serveur.
- Tests dédiés de valeurs, concurrence, effets, erreurs, restauration, doublons,
  réponses API et construction de l'URL de signalement.

La veille physique n'a pas été exécutée sur le Mac de cette session. La
conservation du niveau par le matériel reste une limite connue de la bêta.
La publication de cette fonction désactivée par défaut a été expressément
demandée en acceptant de recueillir les retours terrain.
