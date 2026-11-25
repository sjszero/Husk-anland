// husk-wallpaper-scale.js <src> <dst> <w> <h>
//
// 把壁纸缩放到恰好等于屏幕尺寸（zoom-fill 语义：先按较大的一维放大，再居中裁剪）。
// 为什么需要：见 docs/design/evidence/195-wallpaper-fit-20261125.md —— mutter 用
// Cogl 缩放贴图画背景时，zink/PanVK 这条路上会稳定少画一块，屏幕右侧一块三角里
// 整幅壁纸只有 32% 亮度。壁纸预先做成屏幕尺寸后，mutter 走 1:1 那条路，逐像素吻合。
//
// 退出码：0 写了；3 源图已经是这个尺寸（什么都没写）；其他 失败。
const GdkPixbuf = imports.gi.GdkPixbuf;

// gjs 把脚本自己的路径放在 ARGV[0]，跟 node 的 process.argv 一样
const args = ARGV.slice(ARGV.length > 0 && /\.js$/.test(ARGV[0]) ? 1 : 0);
const [src, dst, wArg, hArg] = args;
const W = parseInt(wArg, 10);
const H = parseInt(hArg, 10);
if (!src || !dst || !(W > 0) || !(H > 0)) {
    printerr('usage: husk-wallpaper-scale.js SRC DST W H');
    imports.system.exit(64);
}

const pb = GdkPixbuf.Pixbuf.new_from_file(src);
const sw0 = pb.get_width();
const sh0 = pb.get_height();
if (sw0 === W && sh0 === H) {
    imports.system.exit(3);
}

const scale = Math.max(W / sw0, H / sh0);
const sw = Math.max(W, Math.round(sw0 * scale));
const sh = Math.max(H, Math.round(sh0 * scale));
const scaled = pb.scale_simple(sw, sh, GdkPixbuf.InterpType.BILINEAR);
const ox = Math.floor((sw - W) / 2);
const oy = Math.floor((sh - H) / 2);
const cut = scaled.new_subpixbuf(ox, oy, W, H);
// copy() 断开对父 pixbuf 的引用，之后 savev 才是确定性的
cut.copy().savev(dst, 'png', [], []);
