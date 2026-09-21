<script setup lang="ts">
import {Check,MonitorCog,Moon,RotateCcw,Sun} from 'lucide-vue-next';
import {accentOptions,useTheme,type AccentName,type ThemeMode} from './theme';
const {preferences,resolvedTheme,setMode,setAccent,reset}=useTheme();
const modes:{id:ThemeMode;label:string;detail:string;icon:typeof Sun}[]=[
  {id:'light',label:'浅色',detail:'明亮工作区',icon:Sun},
  {id:'dark',label:'深色',detail:'适合暗光环境',icon:Moon},
  {id:'system',label:'跟随系统',detail:'随系统外观切换',icon:MonitorCog},
];
</script>

<template>
  <section class="panel settings-panel appearance-panel">
    <div class="appearance-heading"><div><h2><Sun :size="20"/>界面外观</h2><p>主题仅保存在这台设备，不影响素材、任务或 YouTube 设置。</p></div><button class="subtle" @click="reset"><RotateCcw :size="15"/>恢复默认</button></div>
    <fieldset><legend>显示模式</legend><div class="mode-grid"><button v-for="item in modes" :key="item.id" class="mode-option" :class="{selected:preferences.mode===item.id}" :aria-pressed="preferences.mode===item.id" @click="setMode(item.id)"><component :is="item.icon" :size="20"/><span><strong>{{item.label}}</strong><small>{{item.detail}}</small></span><Check v-if="preferences.mode===item.id" :size="16"/></button></div><p v-if="preferences.mode==='system'" class="theme-result">当前系统解析为{{resolvedTheme==='dark'?'深色':'浅色'}}模式</p></fieldset>
    <fieldset><legend>主题色</legend><div class="accent-grid"><button v-for="item in accentOptions" :key="item.id" class="accent-option" :class="{selected:preferences.accent===item.id}" :aria-label="item.label+'主题色'" :aria-pressed="preferences.accent===item.id" @click="setAccent(item.id as AccentName)"><span :style="{background:item.color}"></span><strong>{{item.label}}</strong><Check v-if="preferences.accent===item.id" :size="14"/></button></div></fieldset>
    <label class="motion-option"><input type="checkbox" v-model="preferences.reducedMotion"/><span><strong>减少动态效果</strong><small>关闭旋转和过渡动画，适合对动态敏感的用户。</small></span></label>
  </section>
</template>
