import {computed,ref,watch} from 'vue';

export type ThemeMode='light'|'dark'|'system';
export type AccentName='forest'|'ocean'|'violet'|'amber'|'rose';

export interface ThemePreferences {mode:ThemeMode;accent:AccentName;reducedMotion:boolean}

const STORAGE_KEY='u2bup.appearance.v1';
const modes:ThemeMode[]=['light','dark','system'];
const accents:AccentName[]=['forest','ocean','violet','amber','rose'];
const defaults:ThemePreferences={mode:'system',accent:'forest',reducedMotion:false};

export const accentOptions=[
  {id:'forest' as const,label:'森林',color:'#246c51'},
  {id:'ocean' as const,label:'海洋',color:'#246b92'},
  {id:'violet' as const,label:'紫罗兰',color:'#7054a3'},
  {id:'amber' as const,label:'琥珀',color:'#a86121'},
  {id:'rose' as const,label:'玫瑰',color:'#a64d68'},
];

function readPreferences():ThemePreferences{
  try{
    const stored=JSON.parse(localStorage.getItem(STORAGE_KEY)??'{}') as Partial<ThemePreferences>;
    return {
      mode:modes.includes(stored.mode as ThemeMode)?stored.mode as ThemeMode:defaults.mode,
      accent:accents.includes(stored.accent as AccentName)?stored.accent as AccentName:defaults.accent,
      reducedMotion:stored.reducedMotion===true,
    };
  }catch{return {...defaults};}
}

const preferences=ref<ThemePreferences>(readPreferences());
const systemDark=ref(matchMedia('(prefers-color-scheme: dark)').matches);
export const resolvedTheme=computed<'light'|'dark'>(()=>preferences.value.mode==='system'?(systemDark.value?'dark':'light'):preferences.value.mode);

function applyTheme(){
  const root=document.documentElement;
  root.dataset.theme=resolvedTheme.value;
  root.dataset.accent=preferences.value.accent;
  root.dataset.reduceMotion=preferences.value.reducedMotion?'true':'false';
  root.style.colorScheme=resolvedTheme.value;
}

let initialized=false;
export function initTheme(){
  if(initialized)return;
  initialized=true;
  const query=matchMedia('(prefers-color-scheme: dark)');
  const update=(event:MediaQueryListEvent)=>{systemDark.value=event.matches;applyTheme();};
  query.addEventListener('change',update);
  watch(preferences,value=>{
    try{localStorage.setItem(STORAGE_KEY,JSON.stringify(value));}catch{/* Appearance still works for this session. */}
    applyTheme();
  },{deep:true});
  watch(resolvedTheme,applyTheme);
  applyTheme();
}

export function useTheme(){
  function setMode(mode:ThemeMode){if(modes.includes(mode))preferences.value.mode=mode;}
  function setAccent(accent:AccentName){if(accents.includes(accent))preferences.value.accent=accent;}
  function toggleDark(){preferences.value.mode=resolvedTheme.value==='dark'?'light':'dark';}
  function reset(){preferences.value={...defaults};}
  return {preferences,resolvedTheme,setMode,setAccent,toggleDark,reset};
}
