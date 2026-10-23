(ns jepsen.openraft.scenario.leader-outside-joint
  "Change membership from a joint config through a leader in its old half only
  (#2169).

  With voters V = {A, X, Z}, A appends the joint config [V, V\\{X}] and hands
  leadership to X, which only the old half contains. X is then asked for {Z}.
  X must complete the change even when a bridge through [V\\{X}, {Z}] leaves
  it outside both halves. The cluster must reach the requested stable membership
  and serve writes while preserving the sentinel written before the change."
  (:require [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.openraft [await :as await]
             [client :as http]
             [cluster :as cluster]
             [generator :as guarded]]))

(def recovery-ms 60000)
;; X refuses with these until its own term has a committed entry and no other
;; membership change is in flight; they say nothing about the case under test.
(def ^:private transient-refusals #{:InProgress :UncommittedLeaderLog})
(def ^:private change-timeout-ms 10000)
(def ^:private sentinel-key "leader-outside-joint-sentinel")
(def ^:private fresh-key "leader-outside-joint-fresh")

(defn- roles [test]
  (zipmap [:a :x :z] (:nodes test)))

(defn- ids [& nodes]
  (mapv http/node-host nodes))

(defn- joint-config [test]
  (let [{:keys [a x z]} (roles test)]
    [(ids a x z) (ids a z)]))

(defn- await-status! [test condition pred]
  (await/until!
   condition
   #(let [status (cluster/membership-status test)]
      (if (and status (pred status))
        status
        (await/retry! condition {:membership (dissoc status :metrics)})))
   {:timeout 30000 :retry-interval 100}))

(defn- prepare! [test]
  (let [{:keys [a]} (roles test)
        endpoint (http/api-endpoint test a)
        joint (joint-config test)]
    (http/write! endpoint sentinel-key "before-change")
    ;; append_membership writes exactly this one entry.
    (http/append-membership! endpoint joint)
    (let [status (await-status! test :joint-committed
                                #(= (mapv set joint)
                                    (:committed-voter-configs %)
                                    (:effective-voter-configs %)))]
      {:status :prepared
       :committed-voter-configs (:committed-voter-configs status)})))

(defn- transfer! [test]
  (let [{:keys [a x]} (roles test)]
    (http/transfer-leader! (http/api-endpoint test a) (http/node-host x))
    (let [status (await-status! test :old-half-leader #(= x (:leader %)))]
      {:status :transferred
       :leader (:leader status)
       :effective-voter-configs (:effective-voter-configs status)})))

(defn- change-once! [test]
  (let [{:keys [x z]} (roles test)]
    (try
      (http/change-membership! (http/api-endpoint test x) (ids z))
      {:status :changed}
      (catch Exception e
        (let [{:keys [kind error]} (ex-data e)]
          (cond
            (and (= :openraft-error kind)
                 (contains? error :ChangeMembershipError))
            {:status :refused
             :error-variant (ffirst (:ChangeMembershipError error))}

            ;; X lost leadership after the transfer, so the case was not set up.
            (and (= :openraft-error kind)
                 (contains? error :ForwardToLeader))
            {:status :not-leader}

            ;; A panicking core never answers.
            (#{:request-timeout :transport-error :unreachable} kind)
            {:status :indeterminate
             :kind kind}

            :else
            (throw e)))))))

(defn- transient-refusal? [result]
  (and (= :refused (:status result))
       (contains? transient-refusals (:error-variant result))))

(defn- change!
  "Asks X for {Z}, retrying a refusal that only means X is not ready yet."
  [test]
  (let [attempts (atom 0)
        last-result (atom nil)
        result (try
                 (await/until!
                  :definitive-change-answer
                  #(let [result (change-once! test)]
                     (swap! attempts inc)
                     (reset! last-result result)
                     (if (transient-refusal? result)
                       (await/retry! :definitive-change-answer {:result result})
                       result))
                  {:timeout change-timeout-ms :retry-interval 100})
                 (catch Exception e
                   (if (await/condition-timeout? e :definitive-change-answer)
                     @last-result
                     (throw e))))]
    (assoc result :attempts @attempts)))

(defn- modeled-request-error? [e]
  (let [{:keys [kind error]} (ex-data e)]
    (or (#{:unreachable :request-timeout :transport-error} kind)
        (and (= :openraft-error kind)
             (some #{:ForwardToLeader :QuorumNotEnough :ChangeMembershipError
                     :LogEntryDiscarded}
                   (keys error))))))

(defn- recover!
  "Waits for the requested stable membership, then writes and reads through its
  leader."
  [test]
  (let [{:keys [z]} (roles test)
        voters #{(http/node-host z)}
        started (System/nanoTime)
        elapsed #(quot (- (System/nanoTime) started) 1000000)
        last-observation (atom nil)
        retry! #(await/retry! :leader-outside-joint-recovery @last-observation)]
    (try
      (let [result
            (await/until!
             :leader-outside-joint-recovery
             #(let [status (cluster/membership-status test)
                    endpoint (some->> (:leader status) (http/api-endpoint test))]
                (reset! last-observation {:membership (dissoc status :metrics)})
                (try
                  (cond
                    (nil? status)
                    (retry!)

                    (or (not (:stable? status))
                        (not= voters (:voters status)))
                    (retry!)

                    :else
                    (do
                      (http/write! endpoint fresh-key "after-change")
                      {:leader (:leader status)
                       :voters (:voters status)
                       :fresh (:value (http/linearizable-read! endpoint
                                                               fresh-key))
                       :sentinel (:value (http/linearizable-read!
                                          endpoint
                                          sentinel-key))}))
                  (catch Exception e
                    (if (modeled-request-error? e)
                      (retry!)
                      (throw e)))))
             {:timeout recovery-ms :retry-interval 100})]
        (assoc result :status :recovered :elapsed-ms (elapsed)))
      (catch Exception e
        (if (await/condition-timeout? e :leader-outside-joint-recovery)
          (assoc @last-observation :status :timeout :elapsed-ms (elapsed))
          (throw e))))))

(defn verdict [test history]
  (let [events (filter #(and (= :nemesis (:process %)) (= :info (:type %)))
                       history)
        value-of #(:value (last (filter (fn [op] (= % (:f op))) events)))
        prepared (value-of :prepare-joint)
        transferred (value-of :transfer-to-old-half)
        change (value-of :change-from-old-half)
        recovery (value-of :check-leader-outside-joint-recovery)
        {:keys [x z]} (roles test)
        covered? (and (= :prepared (:status prepared))
                      (= (mapv set (joint-config test))
                         (:committed-voter-configs prepared))
                      (= :transferred (:status transferred))
                      (= x (:leader transferred)))
        changed? (= :changed (:status change))
        recovered? (and (= :recovered (:status recovery))
                        (= #{(http/node-host z)} (:voters recovery))
                        (= "after-change" (:fresh recovery))
                        (= "before-change" (:sentinel recovery)))]
    {:valid? (boolean (and covered? changed? recovered?))
     :scenario-covered? (boolean covered?)
     :changed? (boolean changed?)
     :change change
     :recovered? (boolean recovered?)
     :recovery recovery}))

(defn package []
  {:nemesis
   (reify nemesis/Nemesis
     (setup! [this _test] this)
     (invoke! [_ test op]
       (assoc op :value
              (case (:f op)
                :prepare-joint (prepare! test)
                :transfer-to-old-half (transfer! test)
                :change-from-old-half (change! test)
                :check-leader-outside-joint-recovery (recover! test))))
     (teardown! [_ _test]))
   :checker (reify checker/Checker
              (check [_ test history _] (verdict test history)))})

(defn generator [failure-state]
  (let [op #(gen/once {:type :info :f %})]
    (gen/nemesis
     (guarded/stop-on-harness-failure
      failure-state
      (gen/phases (op :prepare-joint)
                  (op :transfer-to-old-half)
                  (op :change-from-old-half)
                  (op :check-leader-outside-joint-recovery))))))
