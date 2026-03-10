import * as esbuild from 'esbuild';

const watch = process.argv.includes('--watch');

const jsCtx = await esbuild.context({
  entryPoints: ['src/terminal.js', 'src/file_manager.js'],
  bundle: true,
  minify: true,
  outdir: 'dist',
  logLevel: 'info',
});

const cssCtx = await esbuild.context({
  entryPoints: ['src/styles.css'],
  bundle: true,
  minify: true,
  outdir: 'dist',
  logLevel: 'info',
});

if (watch) {
  await jsCtx.watch();
  await cssCtx.watch();
  console.log('Watching for changes…');
} else {
  await jsCtx.rebuild();
  await cssCtx.rebuild();
  await jsCtx.dispose();
  await cssCtx.dispose();
}
