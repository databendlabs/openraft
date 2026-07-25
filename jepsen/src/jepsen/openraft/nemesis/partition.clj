(ns jepsen.openraft.nemesis.partition
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
                    [generator :as gen]
                    [nemesis :as nemesis]
                    [random :as random]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.quorum :as quorum]))

(def partition-seconds 10)
(def recovery-seconds 5)
(def required-partition-modes
  #{:leader-in-majority :leader-in-minority})

(defn- partition-components [nodes configs leader mode]
  (let [voters (quorum/voter-set configs)]
    (when-not (contains? voters leader)
      (throw (ex-info "The current leader is not a voter"
                      {:leader leader
                       :configs configs})))
    (let [quorums (remove #(= voters %)
                          (quorum/quorum-sets configs))
          candidates (case mode
                       :leader-in-majority
                       (filter #(contains? % leader) quorums)

                       :leader-in-minority
                       (remove #(contains? % leader) quorums)

                       (throw (ex-info "Unknown partition mode"
                                       {:mode mode})))
          quorum-side (first (random/shuffle candidates))]
      (when-not quorum-side
        (throw (ex-info "Membership has no partition for the requested mode"
                        {:leader leader
                         :mode mode
                         :configs configs})))
      (let [other-side (remove quorum-side nodes)]
        (if (contains? quorum-side leader)
          [(vec (filter quorum-side nodes)) (vec other-side)]
          [(vec other-side) (vec (filter quorum-side nodes))])))))

(defrecord PartitionNemesis [partitioner]
  nemesis/Nemesis
  (setup! [_ test]
    (PartitionNemesis. (nemesis/setup! partitioner test)))

  (invoke! [_ test op]
    (case (:f op)
      :start-partition
      (let [{:keys [leader] :as status} (cluster/await-ready! test)
            configs (cluster/voter-configs test status)
            mode (:value op)
            components (partition-components (:nodes test)
                                             configs
                                             leader
                                             mode)
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
                       :voter-configs configs
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
