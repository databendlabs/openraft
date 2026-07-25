(ns jepsen.openraft.quorum
  (:require [clojure.set :as set]))

(defn voter-set [configs]
  (reduce set/union #{} (map set configs)))

(defn quorum?
  "True when nodes contain a majority of every voter config."
  [configs nodes]
  (let [nodes (set nodes)]
    (every? (fn [config]
              (let [config (set config)
                    required (inc (quot (count config) 2))]
                (>= (count (set/intersection config nodes))
                    required)))
            configs)))

;; Jepsen clusters are small, so explicit enumeration keeps joint quorum
;; selection simple and directly testable.
(defn- subsets [items]
  (reduce (fn [sets item]
            (into sets (map #(conj % item) sets)))
          [#{}]
          items))

(defn quorum-sets
  "Enumerates every quorum of a stable or joint membership."
  [configs]
  (let [configs (mapv set configs)]
    (when (or (empty? configs)
              (some empty? configs))
      (throw (ex-info "Membership must contain non-empty voter configs"
                      {:configs configs})))
    (->> (voter-set configs)
         subsets
         (filter #(quorum? configs %))
         vec)))

(defn fault-sets
  "Enumerates non-empty voter subsets whose removal leaves a quorum."
  [configs]
  (let [voters (voter-set configs)]
    (->> (quorum-sets configs)
         (map #(set/difference voters %))
         (remove empty?)
         vec)))
