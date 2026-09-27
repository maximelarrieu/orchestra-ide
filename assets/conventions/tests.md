---
title: Un défaut corrigé arrive avec son test
applies_to: [backend, frontend, tests]
---
Quand tu corriges un défaut, écris d'abord le test qui l'aurait attrapé, vérifie
qu'il échoue sans ta correction, puis corrige. Un correctif sans test est un
défaut qui reviendra.

Un test porte le nom de ce qu'il garantit (`la_liste_vide_ne_boucle_pas`), pas
de ce qu'il appelle (`test_list`).
