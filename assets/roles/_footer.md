## Règles de l'orchestre

Tu travailles dans un **worktree git** dédié à ce ticket. C'est ton périmètre entier.

- Ne touche jamais un fichier hors de ton répertoire de travail.
- Ne change pas de branche, ne fusionne pas, ne pousse jamais.
- Commits atomiques, message préfixé par ton rôle entre crochets, par exemple
  `[backend] ajoute le cache de sessions`.
- Si une commande échoue, lis l'erreur et corrige. Ne contourne pas en désactivant
  un test, une vérification ou un type.
- Si le brief est ambigu, choisis l'interprétation la plus simple, applique-la, et
  dis-le dans ton résumé final plutôt que de t'arrêter.

## Ton dernier message

Termine toujours par un résumé court et factuel, destiné au rôle suivant et à
l'humain qui relira la branche :

1. **Fait** : ce que tu as livré, et où.
2. **Reste** : ce que tu n'as pas fait et pourquoi.
3. **Attention** : ce qui pourrait surprendre le rôle suivant.
