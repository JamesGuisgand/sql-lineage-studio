# Limites de l'analyse statique

Le résultat est un modèle *conservateur* : lorsqu'une colonne ne peut pas être reliée de manière certaine, aucune relation fictive n'est produite ; un message est ajouté à `warnings`.

Exemples :

- `SELECT * FROM t` : la liste des colonnes dépend du schéma réel, absent de l'analyse.
- `SELECT id FROM a JOIN b ON ...` : `id` est ambigu sans qualification ou schéma.
- `SELECT COUNT(*) FROM a` : le compte est dérivé des lignes, sans colonne source spécifique.
- Sous-requêtes corrélées dans les expressions et fonctions SQL particulières : les tables référencées peuvent être détectées, mais pas toutes les dépendances de colonnes.
- `UPDATE` et `DELETE` : dépendances de tables, pas des affectations de colonnes.
- SQL dynamique / procédures stockées : non interprétés.
- Des instructions non prises en charge échouent explicitement au lieu de retourner silencieusement un graphe incomplet.

Le parseur AST est une brique **syntaxique**. Sans connexion à une base et sans catalogue de métadonnées, le moteur ne peut ni vérifier l'existence des tables/colonnes ni désambiguïser tous les identifiants.
