<script lang="ts">
  import { mdiFolderSearchOutline, mdiShieldCheckOutline, mdiLightningBoltOutline, mdiMotionPlayOutline } from "@mdi/js";
  import Icon from "./Icon.svelte";
  import Logo from "./Logo.svelte";
  import { addFolder } from "../lib/actions";
  import { isTauri, isMac, openUrl, WEBSITE } from "../lib/api";
</script>

<div class="welcome">
  <div class="hero">
    <Logo size={84} />
    <h1>Your Google Photos, at home on your {isMac ? "Mac" : "computer"}</h1>
    <p class="lede">
      Choose the folder where you unzipped your Google Takeout export, or the external drive that holds it. Takeout
      Viewer reads it in place. Nothing is uploaded, and your files are never changed.
    </p>
    <button class="btn filled big" onclick={addFolder}>
      <Icon path={mdiFolderSearchOutline} size={22} /> Choose Takeout folder
    </button>
    {#if isTauri}<div class="drop">or drop the folder anywhere on this window</div>{/if}
  </div>

  <div class="tips">
    <div class="tip">
      <Icon path={mdiLightningBoltOutline} />
      <div>
        <b>Built for huge libraries</b>
        <span>Hundreds of gigabytes are indexed once; thumbnails are cached so browsing stays instant, even with the drive unplugged.</span>
      </div>
    </div>
    <div class="tip">
      <Icon path={mdiMotionPlayOutline} />
      <div>
        <b>Understands Takeout</b>
        <span>Dates, people, albums and descriptions from the JSON sidecars. Live & motion photos, edits and duplicates are grouped.</span>
      </div>
    </div>
    <div class="tip">
      <Icon path={mdiShieldCheckOutline} />
      <div>
        <b>Safe for old drives</b>
        <span>Read-only, gently paced for external hard drives, and nothing leaves your {isMac ? "Mac" : "computer"}. Unzip all <code>takeout-*.zip</code> parts into one folder; split archives are merged.</span>
      </div>
    </div>
  </div>

  <button class="credit" onclick={() => openUrl(WEBSITE)}>Made by Abhishek Choudhury · abhishekchoudhury.com</button>
</div>

<style>
  .welcome {
    height: 100%;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 48px 32px;
    gap: 56px;
  }
  .hero {
    max-width: 620px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  h1 {
    font-size: 32px;
    font-weight: 400;
    letter-spacing: -0.01em;
    margin: 12px 0 0;
  }
  .lede {
    font-size: 16px;
    color: var(--text-2);
    margin: 0 0 12px;
    line-height: 1.55;
  }
  .big {
    height: 48px;
    padding: 0 28px;
    font-size: 15px;
    border-radius: 24px;
  }
  .drop {
    font-size: 13px;
    color: var(--text-3);
  }
  .tips {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 260px));
    gap: 20px;
  }
  .tip {
    display: flex;
    gap: 14px;
    padding: 18px;
    border-radius: 16px;
    background: var(--surface);
    border: 1px solid var(--outline-soft);
    color: var(--primary);
  }
  .tip div {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-2);
    font-size: 13px;
    line-height: 1.5;
  }
  .tip b {
    color: var(--text);
    font-weight: 500;
    font-size: 14px;
  }
  .credit {
    font-size: 12px;
    color: var(--text-3);
    margin-top: -24px;
  }
  .credit:hover {
    color: var(--primary);
  }
  code {
    font-size: 12px;
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: 4px;
  }
  @media (max-width: 900px) {
    .tips {
      grid-template-columns: 1fr;
    }
  }
</style>
