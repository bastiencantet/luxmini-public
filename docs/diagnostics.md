# Diagnostics consentis

Les parcours et diagnostics de veille utilisent un UUID d'installation aléatoire,
un UUID de session par processus et consentement, et un UUID distinct par cycle.
Le contrat version 2 requiert un nouvel accord explicite ; le refus conserve
les fonctions LED. Aucun compteur activé par défaut n'est inclus.

Les événements de veille indiquent l'option observée active, le début du cycle,
le résultat d'écriture avant la veille et le résultat d'écriture au réveil.
Un résultat accepté par le pilote ne prouve pas l'état physique de la LED.
Les notifications répétées sont ignorées ; plusieurs cycles dans un processus
restent distincts. Aucun niveau de luminosité, horaire, profil matériel, texte
libre d'erreur, compte ou numéro de série n'est envoyé.

La file persistée est bornée à 128 événements. L'activité d'installation précède
les événements, les doublons réseau sont dédupliqués, et les erreurs restent
rejouables. Les timestamps serveur sont des dates de réception. L'historique ne
permet pas de reconstruire une utilisation antérieure à l'instrumentation.

La désactivation efface l'identité et la file locale puis demande l'effacement
serveur. Un nouvel accord ne réutilise ni l'identité, ni les identifiants de
session ou de cycle antérieurs. Les événements d'un cycle entamé sous une ancienne
identité ne passent pas à la nouvelle identité.

Les tables liées sont conservées au maximum 90 jours, avec une purge à 89 jours
pour la marge de planification. Les vues privées de Grafana affichent l'UUID,
les versions et les compteurs d'erreurs. Deux cycles en échec sont nécessaires
pour le panneau de répétitions de veille ; un échec de veille et de réveil dans
un même cycle comptent pour un seul cycle défaillant. L'option observée active
ne constitue pas un état actuel garanti. Les cycles sans retour de réveil sont
inconnus, pas des échecs confirmés.
