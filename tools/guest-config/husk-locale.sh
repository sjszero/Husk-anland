# Husk 的 locale 默认值：中文优先（zh_CN.UTF-8）。
#
# 与 husk-gpu.sh 同一类问题：只写 /etc/default/locale 的话，只有走 PAM 的登录 shell 读得到；
# run-as 起的进程、桌面 fork 出来的程序、env -i 全都拿不到，于是同一个 rootfs 里两套 locale。
# 这里的规则只有一条：程序自己设了 LANG 就不动它，空着才补。
#
# 为什么要补：LANG 为空时 fontconfig 拿不到 zh-CN，`fc-match sans-serif:lang=zh` 会落到
# Noto Sans CJK JP——同一批汉字按日文字形渲染（直、骨、次这类字差别明显）。
case "${LANG:-}" in
    ''|POSIX|C|C.UTF-8) LANG=zh_CN.UTF-8 ;;
esac
export LANG
: "${LANGUAGE:=zh_CN:zh}"
export LANGUAGE
