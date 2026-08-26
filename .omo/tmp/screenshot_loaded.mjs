import { chromium } from 'playwright';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
await page.goto('http://localhost:4173/');
await page.waitForTimeout(1000);
// Find and click the demo load button
const btns = await page.$$('button, a');
let clicked = false;
for (const b of btns) {
  const txt = (await b.textContent()) ?? '';
  if (txt.includes('demo') || txt.includes('Demo') || txt.includes('加载')) {
    await b.click();
    clicked = true;
    break;
  }
}
if (!clicked) {
  // try loading game.json via URL hash
  await page.evaluate(() => { window.location.hash = '#t=500'; });
  await page.reload();
}
await page.waitForTimeout(3000);
// If there's a play/pause, pause it to freeze frame
await page.screenshot({ path: '/tmp/spectator_loaded.png' });
await browser.close();
console.log('loaded screenshot saved');
