# Pacing only: give the GUI frames between steps. Not test synchronization.
p() { sleep 0.03; }
seq 1 100; p
printf '\e[41mred bg\e[K\e[0m\n'; p
printf '\e[44mblue\e[0m \e[7minverse\e[0m \e[1;3;4mbold-it-ul\e[0m \e[9mstrike\e[0m \e[2mdim\e[0m\n'; p
printf '漢字 👍🏽 e\xcc\x81 → ok\n'; p
printf '\e]4;1;rgb:ff/80/00\e\\\e[31mpalette red\e[0m\n'; p
printf '\e]10;rgb:20/ff/20\e\\default fg changed\n'; p
printf '\e]110\e\\'; p
printf '\e[?1049h\e[H\e[2Jalt screen\n'; p
printf '\e[3;8r\e[3H'; for i in $(seq 1 12); do printf 'region %d\n' "$i"; done; p
printf '\e[r\e[2;1H\e[2L\e[1M'; p
printf '\e[45m\e[5;3H\e[10X\e[0m'; p
printf '\e#8'; p
printf '\e[?1049l'; p
printf '\e[10;5H\e[1;42m\e[J\e[0m'; p
printf '\e[2J\e[Hcleared\n'; p
for i in $(seq 1 30); do printf '\e[3%dmline %d %s\e[0m\n' $((i % 8)) "$i" 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; p; done
printf '\e[5;20H\e[?25l'; p; printf '\e[?25h'; p
