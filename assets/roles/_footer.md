## Règles de l'orchestre

Tu travailles dans un **worktree git** dédié à ce ticket. C'est ton périmètre entier.

- Ne touche jamais un fichier hors de ton répertoire de travail.
- Ne change pas de branche, ne fusionne pas, ne pousse jamais.
- Commits atomiques, qui suivent les conventions de l'équipe données plus bas.
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

## Proposer une habitude ou une décision

Si tu as dû redire la même chose qu'aux tickets précédents, ou si tu as tranché
un choix d'architecture que les suivants devront respecter, propose-le. Juste
avant ton résumé (et avant le bloc `VERDICT` si tu en écris un) :

```
PROPOSITION: convention
titre: Toujours paginer les listes d'API
rôles: backend
Le texte de la règle, à l'impératif, tel qu'un agent le lira.
FIN PROPOSITION
```

`convention` pour une habitude d'équipe (et `rôles:` pour dire à qui elle
s'adresse, sinon elle vaut pour tous), `adr` pour une décision d'architecture
(contexte, décision, conséquences). Une proposition n'engage personne tant que
l'humain ne l'a pas acceptée : n'en fais que quand le besoin est réel.
