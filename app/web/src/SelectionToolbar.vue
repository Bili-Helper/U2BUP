<script setup lang="ts">
import {CheckCheck,X} from 'lucide-vue-next';
defineProps<{total:number;matched:number;hidden:number;pageCount:number;scope:string}>();
defineEmits<{clear:[]}>();
</script>
<template>
  <div v-if="total" class="batch-bar selection-toolbar" role="region" :aria-label="scope+'批量操作'">
    <div class="selection-summary"><CheckCheck :size="19"/><strong>已选 {{total}} 项</strong><span>本页 {{pageCount}} · 筛选内 {{matched}} · 筛选外 {{hidden}}</span></div>
    <button class="subtle clear-selection" @click="$emit('clear')"><X :size="16"/>取消全部选择</button>
    <slot/>
  </div>
</template>
