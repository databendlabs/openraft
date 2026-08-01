(ns jepsen.openraft.nemesis
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.cluster :as cluster]))

(def ^:private initial-healthy-seconds 5)

(defrecord RecoveryNemesis []
  nemesis/Nemesis
  (setup! [this _test]
    this)

  (invoke! [_ test op]
    (let [{:keys [leader]} (cluster/await-ready! test)]
      (info "OpenRaft cluster recovered with leader" leader)
      (assoc op :value {:leader leader})))

  (teardown! [this _test]
    this)

  nemesis/Reflection
  (fs [_]
    #{:await-recovery}))

(defn- recovery-package []
  {:nemesis (RecoveryNemesis.)
   :final-generator {:type :info
                     :f :await-recovery}
   :perf #{}})

(defn- fault-class-checker [fault-checker]
  (reify checker/Checker
    (check [_ test history opts]
      (let [result (checker/check fault-checker test history opts)
            executed? (boolean (seq (:observed-modes result)))
            cluster-state (:cluster-state result)
            valid? (cond
                     (not executed?) false
                     (= :intact cluster-state) true
                     (= :unknown cluster-state) :unknown
                     :else false)]
        (-> result
            (dissoc :missing-modes)
            (assoc :valid? valid?
                   :fault-class-executed? executed?))))))

(defn compose-packages
  "Composes fault packages, then confirms recovery after all cleanup."
  [packages]
  (when-not (seq packages)
    (throw (ex-info "At least one nemesis package is required" {})))
  (let [packages (mapv #(assoc % :perf (or (:perf %) #{})) packages)
        composed (combined/compose-packages
                  (conj packages (recovery-package)))
        generator (gen/phases
                   (gen/sleep initial-healthy-seconds)
                   (:generator composed))
        checkers (into {}
                       (map (fn [{:keys [name checker]}]
                              [name (fault-class-checker checker)])
                            packages))]
    (assoc composed
           :generator generator
           :nemesis (nemesis/validate (:nemesis composed))
           :checker (if (= 1 (count packages))
                      (:checker (first packages))
                      (checker/compose checkers)))))
