import { chromium } from 'playwright';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
await page.goto('http://localhost:4173/');
await page.waitForTimeout(2000);
const btns = await page.$$('button');
for (const b of btns) {
  const txt = await b.textContent();
  if (txt && (txt.includes('Load') || txt.includes('demo') || txt.includes('加载'))) {
    await b.click();
    await page.waitForTimeout(2000);
    break;
  }
}
await page.screenshot({ path: '/tmp/spectator_court.png' });
await browser.close();
console.log('screenshot saved');
