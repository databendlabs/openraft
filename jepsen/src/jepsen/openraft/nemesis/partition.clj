(ns jepsen.openraft.nemesis.partition
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
                    [generator :as gen]
                    [nemesis :as nemesis]
                    [random :as random]]
            [jepsen.openraft.cluster :as cluster]))

(def partition-seconds 10)
(def recovery-seconds 5)
(def required-partition-modes
  #{:leader-in-majority :leader-in-minority})

(defn- partition-components [nodes leader mode]
  (let [nodes (vec nodes)
        node-count (count nodes)
        majority-size (inc (quot node-count 2))
        minority-size (- node-count majority-size)]
    (when (< node-count 3)
      (throw (ex-info "A partition test requires at least three nodes"
                      {:nodes nodes})))
    (when-not (some #{leader} nodes)
      (throw (ex-info "The current leader is not a test node"
                      {:leader leader
                       :nodes nodes})))
    (let [leader-component-size (case mode
                                  :leader-in-majority majority-size
                                  :leader-in-minority minority-size
                                  (throw (ex-info "Unknown partition mode"
                                                  {:mode mode})))
          followers (random/shuffle (remove #{leader} nodes))
          leader-component (into [leader]
                                 (take (dec leader-component-size) followers))
          other-component (vec (remove (set leader-component) nodes))]
      [leader-component other-component])))

(defrecord PartitionNemesis [partitioner]
  nemesis/Nemesis
  (setup! [_ test]
    (PartitionNemesis. (nemesis/setup! partitioner test)))

  (invoke! [_ test op]
    (case (:f op)
      :start-partition
      (let [{:keys [leader]} (cluster/await-ready! test)
            mode (:value op)
            components (partition-components (:nodes test) leader mode)
            grudge (nemesis/complete-grudge components)]
        (info "Partitioning OpenRaft nodes"
              {:mode mode
               :leader leader
               :components components})
        (nemesis/invoke! partitioner test
                         (assoc op
                                :f :start
                                :value grudge))
        (assoc op
               :value {:mode mode
                       :leader leader
                       :components components}))

      :stop-partition
      (do
        (info "Healing OpenRaft network partition")
        (nemesis/invoke! partitioner test
                         (assoc op
                                :f :stop
                                :value nil))
        (assoc op :value :network-healed))

      :await-recovery
      (let [{:keys [leader]} (cluster/await-ready! test)]
        (info "OpenRaft cluster recovered with leader" leader)
        (assoc op :value {:leader leader}))))

  (teardown! [_ test]
    (nemesis/teardown! partitioner test)))

(defn partition-nemesis []
  (nemesis/validate
    (PartitionNemesis. (nemesis/partitioner))))

(defn- partition-generator []
  (gen/cycle
    (gen/phases
      (gen/sleep recovery-seconds)
      {:type :info
       :f :start-partition
       :value :leader-in-majority}
      (gen/sleep partition-seconds)
      {:type :info
       :f :stop-partition}
      (gen/sleep recovery-seconds)
      {:type :info
       :f :start-partition
       :value :leader-in-minority}
      (gen/sleep partition-seconds)
      {:type :info
       :f :stop-partition})))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [observed-modes (->> history
                                (filter #(= :start-partition (:f %)))
                                (keep #(get-in % [:value :mode]))
                                set)
            missing-modes (remove observed-modes required-partition-modes)
            recovered? (boolean
                         (some #(and (= :await-recovery (:f %))
                                     (get-in % [:value :leader]))
                               history))]
        {:valid? (and (empty? missing-modes) recovered?)
         :observed-modes (vec (sort observed-modes))
         :missing-modes (vec (sort missing-modes))
         :recovered? recovered?}))))

(defn partition-package []
  {:nemesis (partition-nemesis)
   :generator (partition-generator)
   :final-generator (gen/phases
                      (gen/once {:type :info
                                 :f :stop-partition})
                      (gen/once {:type :info
                                 :f :await-recovery}))
   :checker (coverage-checker)})
