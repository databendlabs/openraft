(ns jepsen.openraft.nemesis.membership
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
                    [generator :as gen]
                    [nemesis :as nemesis]
                    [random :as random]
                    [util :as util]]
            [jepsen.openraft [client :as client]
                             [cluster :as cluster]
                             [db :as openraft-db]]
            [jepsen.openraft.quorum :as quorum]))

(def change-seconds 10)
(def minimum-voters 3)
(def learner-recovery-timeout-ms 60000)

(defn- select-node [test candidates]
  (->> (:nodes test)
       (filter (set candidates))
       random/shuffle
       first))

(defn- voter-ids [test voters]
  (->> (:nodes test)
       (filter (set voters))
       (mapv #(cluster/node-id test %))))

(defn- request-leader! [test leader-endpoint request]
  (client/with-leader!
    leader-endpoint
    (mapv #(client/api-endpoint test %) (:nodes test))
    request))

(defn- ambiguous-request-error? [e]
  (let [{:keys [kind status]} (ex-data e)]
    (or (#{:request-timeout :transport-error :invalid-json} kind)
        (and (= :http-error kind)
             (or (nil? status)
                 (<= 500 status 599))))))

(defn- node-metrics! [test node]
  (try
    (client/metrics! (client/api-endpoint test node))
    (catch Exception e
      (if (= :interrupted (:kind (ex-data e)))
        (throw (InterruptedException. (ex-message e)))
        (throw e)))))

(defn- await-reachable-learner! [test node]
  (try
    (util/await-fn
      #(node-metrics! test node)
      {:log-message (str "Waiting for OpenRaft learner " node)
       :timeout learner-recovery-timeout-ms})
    (catch InterruptedException e
      (.interrupt (Thread/currentThread))
      (throw e))
    (catch Exception e
      (throw (ex-info "Existing OpenRaft learner is unreachable"
                      {:kind :learner-unreachable
                       :node node}
                      e)))))

(defn- request-membership-change!
  [test leader-endpoint target-voters]
  (try
    (request-leader!
      test
      leader-endpoint
      #(client/change-membership! % (voter-ids test target-voters)))
    (catch Exception e
      (when-not (ambiguous-request-error? e)
        (throw e))
      (info "Resolving an ambiguous OpenRaft membership change"
            {:target-voters target-voters
             :error (ex-message e)}))))

(defn- complete-joint-membership!
  [test {:keys [leader effective-voter-configs]}]
  (let [target-voters (peek effective-voter-configs)
        leader-endpoint (atom (client/api-endpoint test leader))]
    (info "Completing an existing joint OpenRaft membership"
          {:configs effective-voter-configs
           :target-voters target-voters})
    (request-membership-change! test leader-endpoint target-voters)
    (cluster/await-stable-membership! test target-voters)))

(defn- stable-membership! [test]
  (let [{:keys [stable?] :as status}
        (cluster/await-committed-membership! test)]
    (if stable?
      status
      (complete-joint-membership! test status))))

(defn- change-membership-and-await!
  [test leader-endpoint target-voters]
  (request-membership-change! test leader-endpoint target-voters)
  (try
    (cluster/await-stable-membership! test target-voters)
    (catch Exception e
      (when-not (= :timeout (:type (ex-data e)))
        (throw e))
      (let [status (stable-membership! test)]
        (if (= target-voters (:voters status))
          status
          (throw e))))))

(defn- await-observed-learner! [test node]
  (util/await-fn
    #(let [status (cluster/membership-status test)]
       (if (and (:stable? status)
                (contains? (:learners status) node))
         status
         (throw (ex-info "OpenRaft learner is not committed yet"
                         {:node node
                          :status status}))))
    {:log-message (str "Waiting for OpenRaft learner " node
                       " to appear in membership")
     :timeout learner-recovery-timeout-ms}))

(defn- add-learner-and-confirm!
  [test leader-endpoint node node-id api-addr raft-addr]
  (try
    (request-leader!
      test
      leader-endpoint
      #(client/add-learner! % node-id api-addr raft-addr))
    (catch Exception e
      (when-not (ambiguous-request-error? e)
        (throw e))
      (info "Resolving an ambiguous OpenRaft learner addition"
            {:node node
             :error (ex-message e)})
      (await-observed-learner! test node))))

(defn- grow! [database test]
  (let [{:keys [leader voters learners non-members metrics]}
        (stable-membership! test)
        source (if (seq learners) :learner :non-member)
        node (select-node test
                          (if (= :learner source)
                            learners
                            non-members))]
    (if-not node
      :membership-full
      (let [leader-endpoint (atom (client/api-endpoint test leader))
            {:keys [node-id api-addr raft-addr]}
            (cluster/node-info test node)
            target-voters (conj voters node)]
        (when (= :non-member source)
          (openraft-db/start-empty-node! database test node))
        (when (and (= :learner source)
                   (not (contains? metrics node)))
          (await-reachable-learner! test node))

        (info "Adding OpenRaft learner"
              {:node node
               :source source
               :leader leader})
        (add-learner-and-confirm!
          test
          leader-endpoint
          node
          node-id
          api-addr
          raft-addr)

        (info "Growing OpenRaft membership"
              {:node node
               :before voters
               :after target-voters})

        (let [final-status
              (change-membership-and-await!
                test
                leader-endpoint
                target-voters)]
          {:node node
           :source source
           :leader (:leader final-status)
           :before voters
           :after (:voters final-status)})))))

(defn- shrink-candidates [{:keys [voters metrics]}]
  (let [reachable (set (keys metrics))]
    (filter
      (fn [node]
        (let [target-voters (disj voters node)]
          (quorum/quorum? [target-voters] reachable)))
      voters)))

(defn- shrink! [database test]
  (let [{:keys [leader voters learners] :as status}
        (stable-membership! test)]
    (cond
      (seq learners)
      :learner-pending

      (<= (count voters) minimum-voters)
      :minimum-membership

      :else
      (if-let [node (select-node test (shrink-candidates status))]
        (let [leader-endpoint (atom (client/api-endpoint test leader))
              target-voters (disj voters node)]
          (info "Shrinking OpenRaft membership"
                {:node node
                 :before voters
                 :after target-voters})

          (let [final-status
                (change-membership-and-await!
                  test
                  leader-endpoint
                  target-voters)]
            (openraft-db/stop-and-wipe-node! database test node)
            {:node node
             :leader (:leader final-status)
             :before voters
             :after (:voters final-status)}))
        :no-quorum-safe-shrink))))

(defn- restore-membership! [database test]
  (let [target-voters (set (:nodes test))]
    (loop [status (stable-membership! test)]
      (if (= target-voters (:voters status))
        (let [ready-status (cluster/await-ready! test)]
          {:leader (:leader ready-status)
           :voters (:voters status)})
        (do
          (grow! database test)
          (recur (stable-membership! test)))))))

(defrecord MembershipNemesis [database]
  nemesis/Nemesis
  (setup! [this _test]
    this)

  (invoke! [_ test op]
    (case (:f op)
      :grow
      (assoc op :value (grow! database test))

      :shrink
      (assoc op :value (shrink! database test))

      :restore-membership
      (assoc op :value (restore-membership! database test))))

  (teardown! [_ _test]))

(defn membership-nemesis [database]
  (nemesis/validate
    (MembershipNemesis. database)))

(defn- membership-generator []
  (gen/stagger
    change-seconds
    (gen/phases
      {:type :info
       :f :shrink}
      {:type :info
       :f :grow}
      (gen/mix [(repeat {:type :info
                         :f :grow})
                (repeat {:type :info
                         :f :shrink})]))))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ test history _opts]
      (let [required-changes #{:grow :shrink}
            expected-voters (set (:nodes test))
            membership-history (->> history
                                    (filter
                                      #(#{:grow
                                          :shrink
                                          :restore-membership} (:f %)))
                                    vec)
            errors (->> membership-history
                        (filter #(or (:error %)
                                     (:exception %)))
                        vec)
            observed-changes (->> history
                                  (filter
                                    #(required-changes (:f %)))
                                  (filter
                                    (fn [op]
                                      (let [value (:value op)]
                                        (and (map? value)
                                             (contains? value :before)
                                             (contains? value :after)
                                             (not= (:before value)
                                                   (:after value))))))
                                  (map :f)
                                  set)
            missing-changes (remove observed-changes
                                    required-changes)
            final-operation (peek membership-history)
            restored? (and (= :restore-membership (:f final-operation))
                           (= expected-voters
                              (get-in final-operation
                                      [:value :voters])))]
        {:valid? (and (empty? errors)
                      (empty? missing-changes)
                      restored?)
         :observed-changes (vec (sort observed-changes))
         :missing-changes (vec (sort missing-changes))
         :restored? restored?
         :count (count errors)
         :errors (vec (take 10 errors))}))))

(defn membership-package [database]
  {:nemesis (membership-nemesis database)
   :generator (membership-generator)
   :final-generator (gen/once {:type :info
                               :f :restore-membership})
   :checker (coverage-checker)})
