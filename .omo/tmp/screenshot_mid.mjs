import { chromium } from 'playwright';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
await page.goto('http://localhost:4173/#t=5000');
await page.waitForTimeout(2000);
// try to find and use the scrub slider
const scrub = await page.$('#scrub');
if (scrub) {
  await page.evaluate(() => {
    const s = document.getElementById('scrub');
    if (s) { s.value = '5000'; s.dispatchEvent(new Event('input', { bubbles: true })); }
  });
  await page.waitForTimeout(1000);
}
// Also try clicking any "demo" load button
const btns = await page.$$('button');
for (const b of btns) {
  const txt = (await b.textContent()) ?? '';
  if (txt.includes('demo') || txt.includes('路径') || txt.includes('加载')) {
    await b.click();
    await page.waitForTimeout(2000);
    // after load, scrub to mid-game
    await page.evaluate(() => {
      const s = document.getElementById('scrub');
      if (s && s.max) { s.value = String(Math.floor(Number(s.max) * 0.3)); s.dispatchEvent(new Event('input', { bubbles: true })); }
    });
    await page.waitForTimeout(500);
    break;
  }
}
await page.screenshot({ path: '/tmp/spectator_mid.png' });
await browser.close();
console.log('mid-game screenshot saved');
