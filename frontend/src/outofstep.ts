import { send } from "./bridge";

const PLUTO_OUT_OF_STEP_TEXTS = [
  "Oopsie!! please refresh your browser and everything will be alright!",
  "عذراً!! يرجى تحديث المتصفح وسيعود كل شيء إلى مساره!",
  "哎呀！！请刷新您的浏览器，一切都会好起来的！",
  "Jejda!! Obnov, prosím, prohlížeč a všechno bude v pořádku!",
  "Ups!! Genindlæs venligst din browser, så skal det nok gå!",
  "Hoppla!! Bitte aktualisiere deinen Browser und alles wird in Ordnung sein!",
  "Ουπς!! παρακαλώ ανανεώστε τον περιηγητή σας και όλα θα είναι εντάξει!",
  "¡¡Ups!! por favor refresca tu navegador y todo estará bien!",
  "Oups !! Merci d'actualiser ton navigateur et tout ira bien !",
  "Ops!! per favore aggiorna il tuo browser e tutto andrà bene!",
  "Oops!! last inn nettleseren din på nytt så blir alt i orden!",
  "ای وای!! لطفاً مرورگر خود را بازخوانی کنید، همه چیز درست خواهد شد!",
  "Upsss! Proszę odświeżyć stronę i wszystko powinno być dobrze!",
  "Oopsie!! Atualiza o teu navegador e tudo ficará bem!",
  "Упс!! пожалуйста, обновите браузер, и всё будет хорошо!",
  "Hups! Päivitä selain ja kaikki on taas kunnossa!",
];

export function isOutOfStep(message: unknown): boolean {
  return PLUTO_OUT_OF_STEP_TEXTS.includes(String(message ?? "").trim());
}

export function wrapAlert(w: { alert: (message?: unknown) => void }, ask: () => void): void {
  const shown = w.alert;
  let asked = false;
  w.alert = function (this: unknown, message?: unknown) {
    if (isOutOfStep(message)) {
      if (!asked) {
        asked = true;
        ask();
      }
      return;
    }
    return shown.call(this, message);
  };
}

export function initOutOfStep() {
  wrapAlert(window as unknown as { alert: (message?: unknown) => void }, () => send({ type: "out_of_step" }));
}
